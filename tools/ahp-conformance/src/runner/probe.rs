//! WebSocket JSON-RPC probe with bounded reads.
//!
//! Internal conformance-runner module.

use std::collections::VecDeque;
use std::time::Duration;

use ahp_types::actions::ActionEnvelope;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::http::header::{ACCEPT, USER_AGENT};
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};
use url::Url;

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
        let socket = if target.starts_with("ws://") {
            Self::connect_plain(target, timeout_duration).await?
        } else {
            Self::connect_standard(target, timeout_duration).await?
        };
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

    async fn connect_standard(
        target: &str,
        timeout_duration: Duration,
    ) -> Result<Ws, ConformanceError> {
        let mut request = target
            .into_client_request()
            .map_err(Self::describe_handshake_error)?;
        request
            .headers_mut()
            .insert(USER_AGENT, HeaderValue::from_static("ahp-conformance/0.1"));
        request
            .headers_mut()
            .insert(ACCEPT, HeaderValue::from_static("*/*"));
        let (socket, _) = timeout(timeout_duration, connect_async(request))
            .await
            .map_err(|_| ConformanceError::Timeout("connecting to WebSocket target".to_owned()))?
            .map_err(Self::describe_handshake_error)?;
        Ok(socket)
    }

    async fn connect_plain(
        target: &str,
        timeout_duration: Duration,
    ) -> Result<Ws, ConformanceError> {
        timeout(timeout_duration, Self::connect_plain_inner(target))
            .await
            .map_err(|_| ConformanceError::Timeout("connecting to WebSocket target".to_owned()))?
    }

    async fn connect_plain_inner(target: &str) -> Result<Ws, ConformanceError> {
        const MAX_RESPONSE_HEADER_BYTES: usize = 16 * 1024;
        let url = Url::parse(target).map_err(|error| {
            ConformanceError::Options(format!("invalid WebSocket URL: {error}"))
        })?;
        let host = url
            .host_str()
            .ok_or_else(|| ConformanceError::Options("WebSocket URL has no host".to_owned()))?;
        let port = url
            .port_or_known_default()
            .ok_or_else(|| ConformanceError::Options("WebSocket URL has no port".to_owned()))?;
        let mut stream = TcpStream::connect((host, port)).await?;
        let key = base64::engine::general_purpose::STANDARD.encode(uuid::Uuid::new_v4().as_bytes());
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host.to_owned()
        };
        let host_header = url
            .port()
            .map_or_else(|| host.clone(), |port| format!("{host}:{port}"));
        let path = url.query().map_or_else(
            || url.path().to_owned(),
            |query| format!("{}?{query}", url.path()),
        );
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {host_header}\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: {key}\r\nUser-Agent: ahp-conformance/0.1\r\nAccept: */*\r\n\r\n"
        );
        stream.write_all(request.as_bytes()).await?;
        stream.flush().await?;

        let mut received = Vec::new();
        let header_end = loop {
            let mut chunk = [0_u8; 1024];
            let count = stream.read(&mut chunk).await?;
            if count == 0 {
                return Err(ConformanceError::Closed);
            }
            received.extend_from_slice(&chunk[..count]);
            if let Some(header_end) = bounded_header_end(&received, MAX_RESPONSE_HEADER_BYTES)? {
                break header_end;
            }
        };
        let mut headers = [httparse::EMPTY_HEADER; 32];
        let mut response = httparse::Response::new(&mut headers);
        response.parse(&received[..header_end]).map_err(|error| {
            ConformanceError::Options(format!("invalid WebSocket handshake response: {error}"))
        })?;
        let status = response.code.ok_or_else(|| {
            ConformanceError::Options("WebSocket handshake response omitted a status".to_owned())
        })?;
        if status != 101 {
            let body = String::from_utf8_lossy(&received[header_end..]);
            let body = body.trim();
            return Err(ConformanceError::Options(if body.is_empty() {
                format!("WebSocket handshake returned HTTP {status}")
            } else {
                format!("WebSocket handshake returned HTTP {status}: {body}")
            }));
        }
        Self::validate_upgrade_headers(response.headers, &key)?;
        let unread = received.split_off(header_end);
        Ok(WebSocketStream::from_partially_read(
            MaybeTlsStream::Plain(stream),
            unread,
            Role::Client,
            None,
        )
        .await)
    }

    fn validate_upgrade_headers(
        headers: &[httparse::Header<'_>],
        key: &str,
    ) -> Result<(), ConformanceError> {
        let has_token = |name: &str, expected: &str| {
            headers
                .iter()
                .filter(|header| header.name.eq_ignore_ascii_case(name))
                .filter_map(|header| std::str::from_utf8(header.value).ok())
                .flat_map(|value| value.split(','))
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        };
        let expected = derive_accept_key(key.as_bytes());
        let accept = headers
            .iter()
            .filter(|header| header.name.eq_ignore_ascii_case("sec-websocket-accept"))
            .filter_map(|header| std::str::from_utf8(header.value).ok())
            .any(|value| value.trim() == expected);
        if has_token("upgrade", "websocket") && has_token("connection", "upgrade") && accept {
            Ok(())
        } else {
            Err(ConformanceError::Options(
                "WebSocket handshake returned invalid upgrade headers".to_owned(),
            ))
        }
    }

    fn describe_handshake_error(error: tokio_tungstenite::tungstenite::Error) -> ConformanceError {
        let tokio_tungstenite::tungstenite::Error::Http(response) = error else {
            return ConformanceError::Transport(error);
        };
        let body = response
            .body()
            .as_deref()
            .and_then(|body| std::str::from_utf8(body).ok())
            .map(str::trim)
            .filter(|body| !body.is_empty());
        ConformanceError::Options(match body {
            Some(body) => format!(
                "WebSocket handshake returned HTTP {}: {body}",
                response.status()
            ),
            None => format!("WebSocket handshake returned HTTP {}", response.status()),
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
            let error = message
                .get("error")
                .map_or_else(|| "no error object".to_owned(), Value::to_string);
            ConformanceError::Options(format!(
                "{operation} did not return a JSON-RPC result: {error}"
            ))
        })?;
        serde_json::from_value(result).map_err(ConformanceError::from)
    }

    pub(crate) async fn dispatch_action(
        &mut self,
        channel: &str,
        action: Value,
    ) -> Result<(), ConformanceError> {
        self.dispatch_action_with_sequence(channel, action)
            .await
            .map(|_| ())
    }

    pub(crate) async fn dispatch_action_with_sequence(
        &mut self,
        channel: &str,
        action: Value,
    ) -> Result<i64, ConformanceError> {
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
        .await?;
        Ok(client_seq)
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

fn bounded_header_end(received: &[u8], maximum: usize) -> Result<Option<usize>, ConformanceError> {
    let header_end = received
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4);
    if header_end.is_some_and(|end| end > maximum)
        || (header_end.is_none() && received.len() >= maximum)
    {
        return Err(ConformanceError::Options(
            "WebSocket handshake response headers exceeded 16 KiB".to_owned(),
        ));
    }
    Ok(header_end)
}

#[cfg(test)]
mod handshake_tests {
    use super::{bounded_header_end, Probe};

    #[test]
    fn accepts_upgrade_token_split_across_connection_headers() {
        let headers = [
            httparse::Header {
                name: "Upgrade",
                value: b"websocket",
            },
            httparse::Header {
                name: "Connection",
                value: b"keep-alive",
            },
            httparse::Header {
                name: "Connection",
                value: b"Upgrade",
            },
            httparse::Header {
                name: "Sec-WebSocket-Accept",
                value: b"s3pPLMBiTxaQ9kYGzzhZRbK+xOo=",
            },
        ];
        assert!(Probe::validate_upgrade_headers(&headers, "dGhlIHNhbXBsZSBub25jZQ==").is_ok());
    }

    #[test]
    fn rejects_a_header_terminator_beyond_the_limit() {
        let mut oversized = vec![b'a'; 16];
        oversized.extend_from_slice(b"\r\n\r\n");
        assert!(bounded_header_end(&oversized, 16).is_err());
        assert_eq!(
            bounded_header_end(b"HTTP/1.1 101\r\n\r\nbody", 16).expect("bounded headers"),
            Some(16)
        );
    }
}
