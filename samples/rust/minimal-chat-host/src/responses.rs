// Flow: POST one user input to Responses, decode SSE text, and finish its turn.

use futures_util::StreamExt;
use reqwest::Url;
use serde_json::{json, Value};

use crate::host::{AppState, FinishedTurn, HostError, HostResult, CHAT_URI};

const RETAINED_TURNS: usize = 8;

#[derive(Debug, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

#[derive(Default)]
pub struct SseDecoder {
    line: Vec<u8>,
    event: Option<String>,
    data: Vec<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ResponsesEvent {
    Started(String),
    TextDelta(String),
    Completed,
    Failed(String),
    Ignored,
}

pub(crate) fn endpoint(base_url: &str) -> HostResult<Url> {
    let mut base = Url::parse(base_url)
        .map_err(|error| HostError(format!("--openai-base-url is not a URL: {error}")))?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        return Err(HostError(
            "--openai-base-url must be an http:// or https:// API origin".to_owned(),
        ));
    }
    if base.path() != "/" && !base.path().is_empty() {
        return Err(HostError(
            "--openai-base-url must be an API origin, without a path such as /v1".to_owned(),
        ));
    }
    base.set_query(None);
    base.set_fragment(None);
    base.join("v1/responses")
        .map_err(|error| HostError(format!("could not build /v1/responses URL: {error}")))
}

pub(crate) async fn run_turn(
    state: AppState,
    turn_id: String,
    prompt: String,
    history: Vec<FinishedTurn>,
    previous_response_id: Option<String>,
) {
    let outcome = stream_text(
        &state,
        &turn_id,
        &prompt,
        &history,
        previous_response_id.as_deref(),
    )
    .await;
    let (state_name, error, action_type, response_id) = match outcome {
        Ok(response_id) => ("complete", None, "chat/turnComplete", response_id),
        Err(error) => ("error", Some(error), "chat/error", None),
    };
    let _ = finish_turn(
        &state,
        &turn_id,
        state_name,
        error,
        action_type,
        None,
        response_id,
    )
    .await;
}

async fn stream_text(
    state: &AppState,
    turn_id: &str,
    prompt: &str,
    history: &[FinishedTurn],
    previous_response_id: Option<&str>,
) -> Result<Option<String>, String> {
    let chained = state.config.response_chaining && previous_response_id.is_some();
    let input = if chained {
        vec![json!({ "role": "user", "content": prompt })]
    } else {
        model_input(history, prompt)
    };
    let mut body = json!({
        "model": state.config.model,
        "stream": true,
        "input": input,
    });
    if state.config.response_chaining {
        body.as_object_mut()
            .ok_or_else(|| "Responses request body was not an object".to_owned())?
            .insert("store".to_owned(), Value::Bool(true));
    }
    if chained {
        body.as_object_mut()
            .ok_or_else(|| "Responses request body was not an object".to_owned())?
            .insert(
                "previous_response_id".to_owned(),
                json!(previous_response_id),
            );
    }
    let mut request = state
        .http
        .post(state.config.responses_url.clone())
        .json(&body);
    if let Some(api_key) = &state.config.api_key {
        request = request.bearer_auth(api_key);
    }

    let response = request.send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.map_err(|error| error.to_string())?;
        return Err(format!("Responses API returned {status}: {body}"));
    }

    let mut decoder = SseDecoder::default();
    let mut body = response.bytes_stream();
    let mut response_id = None;
    while let Some(chunk) = body.next().await {
        for event in decoder.push(&chunk.map_err(|error| error.to_string())?)? {
            match parse_responses_event(&event)? {
                ResponsesEvent::Started(id) => {
                    if response_id.replace(id).is_some() {
                        return Err("Responses stream identified itself more than once".to_owned());
                    }
                }
                ResponsesEvent::TextDelta(content) => append_delta(state, turn_id, content).await,
                ResponsesEvent::Completed => {
                    if state.config.response_chaining && response_id.is_none() {
                        return Err(
                            "Responses stream omitted the response id required for chaining"
                                .to_owned(),
                        );
                    }
                    return Ok(response_id);
                }
                ResponsesEvent::Failed(message) => return Err(message),
                ResponsesEvent::Ignored => {}
            }
        }
    }
    Err("Responses SSE stream ended before response.completed".to_owned())
}

fn model_input(history: &[FinishedTurn], prompt: &str) -> Vec<Value> {
    let mut input = Vec::with_capacity(history.len().saturating_mul(2).saturating_add(1));
    for turn in history {
        if let Some(user) = turn.message.get("text").and_then(Value::as_str) {
            input.push(json!({ "role": "user", "content": user }));
        }
        if !turn.response.is_empty() {
            input.push(json!({ "role": "assistant", "content": turn.response }));
        }
    }
    input.push(json!({ "role": "user", "content": prompt }));
    input
}

async fn append_delta(state: &AppState, turn_id: &str, content: String) {
    if content.is_empty() {
        return;
    }
    let _publication = state.publication.lock().await;
    {
        let mut shared = state.shared.lock().await;
        let Some(active) = shared.active.as_mut().filter(|active| active.id == turn_id) else {
            return;
        };
        active.response.push_str(&content);
        shared.modified_at = crate::host::timestamp();
    }
    state
        .publish_locked(
            CHAT_URI,
            json!({ "type": "chat/delta", "turnId": turn_id, "partId": format!("{turn_id}/text"), "content": content }),
            None,
            None,
        )
        .await;
}

pub(crate) async fn finish_turn(
    state: &AppState,
    turn_id: &str,
    outcome: &'static str,
    error: Option<String>,
    action_type: &'static str,
    origin: Option<Value>,
    response_id: Option<String>,
) -> Option<futures_util::future::AbortHandle> {
    let _publication = state.publication.lock().await;
    let (duration, abort) = {
        let mut shared = state.shared.lock().await;
        if shared.active.as_ref().map(|active| active.id.as_str()) != Some(turn_id) {
            return None;
        }
        let active = shared.active.take()?;
        let duration = active
            .started
            .elapsed()
            .as_millis()
            .try_into()
            .unwrap_or(i64::MAX);
        shared.modified_at = crate::host::timestamp();
        shared.turns.push(FinishedTurn {
            id: active.id,
            started_at: active.started_at,
            duration,
            message: active.message,
            response: active.response,
            state: outcome,
            error: error.clone(),
        });
        if shared.turns.len() > RETAINED_TURNS {
            shared.turns.remove(0);
        }
        shared.response_id = if outcome == "complete" {
            response_id
        } else {
            None
        };
        (duration, active.abort)
    };
    let action = if let Some(error) = error {
        json!({
            "type": action_type,
            "turnId": turn_id,
            "duration": duration,
            "part": {
                "kind": "error",
                "error": { "errorType": "upstream", "message": error },
            },
        })
    } else {
        json!({ "type": action_type, "turnId": turn_id, "duration": duration })
    };
    state.publish_locked(CHAT_URI, action, origin, None).await;
    Some(abort)
}

impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<SseEvent>, String> {
        let mut events = Vec::new();
        for byte in bytes {
            if *byte == b'\n' {
                let mut line = std::mem::take(&mut self.line);
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if let Some(event) = self.parse_line(&line)? {
                    events.push(event);
                }
            } else {
                self.line.push(*byte);
            }
        }
        Ok(events)
    }

    fn parse_line(&mut self, line: &[u8]) -> Result<Option<SseEvent>, String> {
        if line.is_empty() {
            let event = (!self.data.is_empty()).then(|| SseEvent {
                event: self.event.take(),
                data: std::mem::take(&mut self.data).join("\n"),
            });
            self.event = None;
            return Ok(event);
        }
        if line.first() == Some(&b':') {
            return Ok(None);
        }
        let line = std::str::from_utf8(line)
            .map_err(|error| format!("Responses SSE contained non-UTF-8 data: {error}"))?;
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = (!value.is_empty()).then(|| value.to_owned()),
            "data" => self.data.push(value.to_owned()),
            _ => {}
        }
        Ok(None)
    }
}

pub fn parse_responses_event(event: &SseEvent) -> Result<ResponsesEvent, String> {
    let value: Value = serde_json::from_str(&event.data)
        .map_err(|error| format!("Responses SSE data was not JSON: {error}"))?;
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .or(event.event.as_deref())
        .ok_or_else(|| "Responses SSE event had no type".to_owned())?;
    match event_type {
        "response.created" => match value.pointer("/response/id") {
            Some(Value::String(id)) if !id.is_empty() => Ok(ResponsesEvent::Started(id.clone())),
            Some(Value::String(_)) => {
                Err("response.created contained an empty response id".to_owned())
            }
            Some(_) => Err("response.created response id was not a string".to_owned()),
            None => Ok(ResponsesEvent::Ignored),
        },
        "response.output_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|delta| ResponsesEvent::TextDelta(delta.to_owned()))
            .ok_or_else(|| "response.output_text.delta lacked a string delta".to_owned()),
        "response.completed" => Ok(ResponsesEvent::Completed),
        "response.failed" | "response.incomplete" | "error" => {
            Ok(ResponsesEvent::Failed(failure_message(&value)))
        }
        _ => Ok(ResponsesEvent::Ignored),
    }
}

fn failure_message(value: &Value) -> String {
    value
        .pointer("/error/message")
        .or_else(|| value.pointer("/response/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Responses API reported a failed or incomplete response")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{model_input, FinishedTurn};

    #[test]
    fn model_input_includes_retained_turns() {
        let history = vec![FinishedTurn {
            id: "first".into(),
            started_at: "2026-09-02T00:00:00Z".into(),
            duration: 1,
            message: json!({ "text": "first question", "origin": { "kind": "user" } }),
            response: "first answer".into(),
            state: "complete",
            error: None,
        }];

        assert_eq!(
            model_input(&history, "second question"),
            vec![
                json!({ "role": "user", "content": "first question" }),
                json!({ "role": "assistant", "content": "first answer" }),
                json!({ "role": "user", "content": "second question" }),
            ]
        );
    }
}
