//! WebSocket JSON-RPC probe with bounded reads.
//!
//! Internal conformance-runner module.

use std::collections::VecDeque;
use std::time::Duration;

use ahp_types::actions::ActionEnvelope;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use super::ConformanceError;

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub(crate) struct Probe {
    writer: futures_util::stream::SplitSink<Ws, Message>,
    reader: futures_util::stream::SplitStream<Ws>,
    buffered: VecDeque<Value>,
    next_id: u64,
    next_client_seq: i64,
    timeout: Duration,
    last_server_seq: Option<i64>,
}

impl Probe {
    pub(crate) async fn connect(
        target: &str,
        timeout_duration: Duration,
    ) -> Result<Self, ConformanceError> {
        let (socket, _) = timeout(timeout_duration, connect_async(target))
            .await
            .map_err(|_| ConformanceError::Timeout(format!("connecting to {target}")))??;
        let (writer, reader) = socket.split();
        Ok(Self {
            writer,
            reader,
            buffered: VecDeque::new(),
            next_id: 1,
            next_client_seq: 1,
            timeout: timeout_duration,
            last_server_seq: None,
        })
    }

    pub(crate) async fn initialize(
        &mut self,
        client_id: &str,
        protocol_versions: Vec<&str>,
    ) -> Result<Value, ConformanceError> {
        let versions = protocol_versions
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        self.request(
            "initialize",
            json!({
                "channel": "ahp-root://",
                "protocolVersions": versions,
                "clientId": client_id,
                "initialSubscriptions": ["ahp-root://"],
            }),
        )
        .await
    }

    pub(crate) async fn request(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<Value, ConformanceError> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.send_value(json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))
        .await?;
        loop {
            let message = self.next_value().await?;
            if message.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(message);
            }
            self.buffered.push_back(message);
        }
    }

    pub(crate) async fn notify(
        &mut self,
        method: &str,
        params: Value,
    ) -> Result<(), ConformanceError> {
        self.send_value(json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
        .await
    }

    pub(crate) fn decode_result<T: DeserializeOwned>(
        message: &Value,
        operation: &str,
    ) -> Result<T, ConformanceError> {
        let result = message.get("result").cloned().ok_or_else(|| {
            ConformanceError::Options(format!("{operation} did not return a JSON-RPC result"))
        })?;
        serde_json::from_value(result).map_err(ConformanceError::from)
    }

    pub(crate) async fn dispatch_action(
        &mut self,
        channel: &str,
        action: Value,
    ) -> Result<(), ConformanceError> {
        let client_seq = self.next_client_seq;
        self.next_client_seq = self.next_client_seq.saturating_add(1);
        self.notify(
            "dispatchAction",
            json!({
                "channel": channel,
                "clientSeq": client_seq,
                "action": action,
            }),
        )
        .await
    }

    pub(crate) async fn send_text(&mut self, text: &str) -> Result<Value, ConformanceError> {
        self.writer
            .send(Message::Text(text.to_owned().into()))
            .await?;
        self.wait_for(|message| message.get("error").is_some())
            .await
    }

    async fn send_value(&mut self, value: Value) -> Result<(), ConformanceError> {
        let wire = serde_json::to_string(&value)?;
        self.writer.send(Message::Text(wire.into())).await?;
        Ok(())
    }

    pub(crate) async fn wait_action(
        &mut self,
        channel: &str,
        action_type: &str,
    ) -> Result<Value, ConformanceError> {
        self.wait_action_any(channel, &[action_type]).await
    }

    pub(crate) async fn wait_action_any(
        &mut self,
        channel: &str,
        action_types: &[&str],
    ) -> Result<Value, ConformanceError> {
        let message = self
            .wait_for(|message| {
                message.get("method").and_then(Value::as_str) == Some("action")
                    && message.pointer("/params/channel").and_then(Value::as_str) == Some(channel)
                    && message
                        .pointer("/params/action/type")
                        .and_then(Value::as_str)
                        .is_some_and(|action_type| action_types.contains(&action_type))
            })
            .await
            .map_err(|error| self.describe_action_timeout(error, channel, action_types))?;
        let params = message
            .get("params")
            .cloned()
            .ok_or_else(|| ConformanceError::Options("action omitted params".to_owned()))?;
        let _: ActionEnvelope = serde_json::from_value(params)?;
        Ok(message)
    }

    fn describe_action_timeout(
        &self,
        error: ConformanceError,
        channel: &str,
        action_types: &[&str],
    ) -> ConformanceError {
        let ConformanceError::Timeout(_) = error else {
            return error;
        };
        let buffered = self
            .buffered
            .iter()
            .filter_map(|message| {
                let channel = message.pointer("/params/channel")?.as_str()?;
                let action = message.pointer("/params/action/type")?.as_str()?;
                Some(format!("{channel}:{action}"))
            })
            .collect::<Vec<_>>();
        ConformanceError::Timeout(format!(
            "waiting for {} on {channel}; buffered actions: {}",
            action_types.join(" or "),
            if buffered.is_empty() {
                "none".to_owned()
            } else {
                buffered.join(", ")
            }
        ))
    }

    async fn wait_for<F>(&mut self, matches: F) -> Result<Value, ConformanceError>
    where
        F: Fn(&Value) -> bool,
    {
        if let Some(index) = self.buffered.iter().position(&matches) {
            return self.buffered.remove(index).ok_or_else(|| {
                ConformanceError::Options("buffer changed unexpectedly".to_owned())
            });
        }
        loop {
            let message = self.next_value().await?;
            if matches(&message) {
                return Ok(message);
            }
            self.buffered.push_back(message);
        }
    }

    async fn next_value(&mut self) -> Result<Value, ConformanceError> {
        loop {
            let frame = timeout(self.timeout, self.reader.next())
                .await
                .map_err(|_| ConformanceError::Timeout("waiting for an AHP frame".to_owned()))?
                .ok_or(ConformanceError::Closed)??;
            match frame {
                Message::Text(text) => {
                    let value = serde_json::from_str(text.as_str())?;
                    self.observe_server_sequence(&value)?;
                    return Ok(value);
                }
                Message::Binary(bytes) => {
                    let value = serde_json::from_slice(&bytes)?;
                    self.observe_server_sequence(&value)?;
                    return Ok(value);
                }
                Message::Close(_) => return Err(ConformanceError::Closed),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    }

    pub(crate) async fn close(&mut self) -> Result<(), ConformanceError> {
        self.writer.close().await?;
        Ok(())
    }

    pub(crate) async fn assert_no_post_cancel_activity_for_turn(
        &mut self,
        channel: &str,
        turn_id: &str,
        cancelled_server_seq: i64,
        duration: Duration,
    ) -> Result<(), ConformanceError> {
        if let Some(action_type) = self.buffered.iter().find_map(|message| {
            action_after(message, cancelled_server_seq)
                .then(|| post_cancel_activity_type(message, channel, turn_id))
                .flatten()
        }) {
            return Err(ConformanceError::Options(format!(
                "received post-cancellation {action_type} for turn {turn_id:?}"
            )));
        }
        let deadline = tokio::time::Instant::now() + duration;
        loop {
            let frame = match tokio::time::timeout_at(deadline, self.reader.next()).await {
                Ok(Some(frame)) => frame?,
                Ok(None) => return Err(ConformanceError::Closed),
                Err(_) => return Ok(()),
            };
            let message = match frame {
                Message::Text(text) => serde_json::from_str(text.as_str())?,
                Message::Binary(bytes) => serde_json::from_slice(&bytes)?,
                Message::Close(_) => return Err(ConformanceError::Closed),
                Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => continue,
            };
            self.observe_server_sequence(&message)?;
            if let Some(action_type) = action_after(&message, cancelled_server_seq)
                .then(|| post_cancel_activity_type(&message, channel, turn_id))
                .flatten()
            {
                return Err(ConformanceError::Options(format!(
                    "received post-cancellation {action_type} for turn {turn_id:?}"
                )));
            }
            self.buffered.push_back(message);
        }
    }

    fn observe_server_sequence(&mut self, message: &Value) -> Result<(), ConformanceError> {
        if message.get("method").and_then(Value::as_str) != Some("action") {
            return Ok(());
        }
        let sequence = Self::action_server_sequence(message)?;
        if self
            .last_server_seq
            .is_some_and(|previous| sequence <= previous)
        {
            return Err(ConformanceError::Options(format!(
                "serverSeq {sequence} is not greater than the previously observed action"
            )));
        }
        self.last_server_seq = Some(sequence);
        Ok(())
    }

    pub(crate) fn action_server_sequence(message: &Value) -> Result<i64, ConformanceError> {
        message
            .pointer("/params/serverSeq")
            .and_then(Value::as_i64)
            .ok_or_else(|| ConformanceError::Options("action omitted integer serverSeq".to_owned()))
    }
}

fn action_after(message: &Value, server_seq: i64) -> bool {
    message
        .pointer("/params/serverSeq")
        .and_then(Value::as_i64)
        .is_some_and(|sequence| sequence > server_seq)
}

fn post_cancel_activity_type<'a>(
    message: &'a Value,
    channel: &str,
    turn_id: &str,
) -> Option<&'a str> {
    if message.get("method").and_then(Value::as_str) != Some("action")
        || message.pointer("/params/channel").and_then(Value::as_str) != Some(channel)
        || message
            .pointer("/params/action/turnId")
            .and_then(Value::as_str)
            != Some(turn_id)
    {
        return None;
    }
    let action_type = message
        .pointer("/params/action/type")
        .and_then(Value::as_str)?;
    (matches!(
        action_type,
        "chat/delta" | "chat/error" | "chat/turnComplete"
    ) || action_type.starts_with("chat/toolCall"))
    .then_some(action_type)
}
