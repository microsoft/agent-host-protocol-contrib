use std::time::Duration;

use deterministic_openai::ObservedApi;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::local_stack::LocalStack;
use super::super::{ConformanceError, Probe};

pub(super) const DEFAULT_MODEL: &str = "fixture-model";
pub(super) const FAST_MODEL: &str = "fixture-fast";
pub(super) const REASONING_MODEL: &str = "fixture-reasoning";

pub(super) struct SessionChat {
    pub(super) session: String,
    pub(super) chat: String,
}

pub(super) fn model_selection(id: &str, thinking_level: Option<&str>) -> Value {
    match thinking_level {
        Some(thinking_level) => json!({
            "id": id,
            "config": { "thinkingLevel": thinking_level },
        }),
        None => json!({ "id": id }),
    }
}

pub(super) fn session_config(id: &str, thinking_level: Option<&str>) -> Value {
    json!({ "model": model_selection(id, thinking_level) })
}

pub(super) fn require_result<'a>(
    reply: &'a Value,
    operation: &str,
) -> Result<&'a Value, ConformanceError> {
    if let Some(error) = reply.get("error") {
        return Err(ConformanceError::Options(format!(
            "{operation} returned JSON-RPC error {error}"
        )));
    }
    reply.get("result").ok_or_else(|| {
        ConformanceError::Options(format!("{operation} omitted its JSON-RPC result"))
    })
}

pub(super) fn require_error(reply: &Value, operation: &str) -> Result<(), ConformanceError> {
    let error = reply
        .get("error")
        .and_then(Value::as_object)
        .ok_or_else(|| ConformanceError::Options(format!("{operation} unexpectedly succeeded")))?;
    if !error.get("code").is_some_and(Value::is_i64)
        || !error
            .get("message")
            .and_then(Value::as_str)
            .is_some_and(|message| !message.is_empty())
    {
        return Err(ConformanceError::Options(format!(
            "{operation} returned a malformed JSON-RPC error"
        )));
    }
    Ok(())
}

pub(super) async fn create_session(
    probe: &mut Probe,
    suffix: &str,
    config: Value,
) -> Result<SessionChat, ConformanceError> {
    let session = format!("ahp-session:/{}-{}", suffix, Uuid::new_v4());
    let created = probe
        .request(
            "createSession",
            json!({
                "channel": &session,
                "provider": "openai",
                "config": config,
            }),
        )
        .await?;
    require_result(&created, "createSession")?;
    let state = session_state(probe, &session).await?;
    let chat = state
        .get("defaultChat")
        .and_then(Value::as_str)
        .filter(|chat| chat.starts_with("ahp-chat:/"))
        .ok_or_else(|| ConformanceError::Options("configured session has no default chat".into()))?
        .to_owned();
    let subscribed = probe
        .request("subscribe", json!({ "channel": &chat }))
        .await?;
    require_result(&subscribed, "chat subscribe")?;
    Ok(SessionChat { session, chat })
}

pub(super) async fn session_state(
    probe: &mut Probe,
    session: &str,
) -> Result<Value, ConformanceError> {
    let subscribed = probe
        .request("subscribe", json!({ "channel": session }))
        .await?;
    require_result(&subscribed, "session subscribe")?
        .pointer("/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("session subscribe omitted snapshot state".into()))
}

pub(super) async fn chat_state(probe: &mut Probe, chat: &str) -> Result<Value, ConformanceError> {
    let subscribed = probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    require_result(&subscribed, "chat subscribe")?
        .pointer("/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("chat subscribe omitted snapshot state".into()))
}

pub(super) fn assert_session_config(
    state: &Value,
    expected: &Value,
) -> Result<(), ConformanceError> {
    if state.pointer("/config/values/model") != Some(expected) {
        return Err(ConformanceError::Options(format!(
            "session config values.model was {:?}, expected {expected}",
            state.pointer("/config/values/model")
        )));
    }
    let model = state
        .pointer("/config/schema/properties/model")
        .and_then(Value::as_object)
        .ok_or_else(|| ConformanceError::Options("session config schema omitted model".into()))?;
    if model.get("type").and_then(Value::as_str) != Some("object")
        || model.get("sessionMutable").and_then(Value::as_bool) != Some(true)
        || model
            .pointer("/properties/id/enum")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
    {
        return Err(ConformanceError::Options(
            "session config schema did not expose a mutable model selection".into(),
        ));
    }
    Ok(())
}

pub(super) async fn dispatch_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    prompt: String,
    model: Option<Value>,
) -> Result<(), ConformanceError> {
    let mut message = json!({ "text": prompt, "origin": { "kind": "user" } });
    if let Some(model) = model {
        message
            .as_object_mut()
            .expect("JSON object literal")
            .insert("model".into(), model);
    }
    probe
        .dispatch_action(
            chat,
            json!({
                "type": "chat/turnStarted",
                "turnId": turn_id,
                "startedAt": "2026-09-01T00:00:00.000Z",
                "message": message,
            }),
        )
        .await
}

pub(super) async fn complete_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    prompt: String,
    model: Option<Value>,
) -> Result<Value, ConformanceError> {
    dispatch_turn(probe, chat, turn_id, prompt, model).await?;
    let started = probe.wait_action(chat, "chat/turnStarted").await?;
    assert_action_turn(&started, turn_id, "chat/turnStarted")?;
    let terminal = probe
        .wait_action_any(chat, &["chat/turnComplete", "chat/error"])
        .await?;
    assert_action_turn(&terminal, turn_id, "turn terminal")?;
    if terminal
        .pointer("/params/action/type")
        .and_then(Value::as_str)
        != Some("chat/turnComplete")
    {
        return Err(ConformanceError::Options(format!(
            "turn {turn_id:?} ended with {:?}",
            terminal.pointer("/params/action/error")
        )));
    }
    Ok(started)
}

pub(super) fn assert_action_turn(
    action: &Value,
    turn_id: &str,
    operation: &str,
) -> Result<(), ConformanceError> {
    if action
        .pointer("/params/action/turnId")
        .and_then(Value::as_str)
        == Some(turn_id)
    {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "{operation} did not target turn {turn_id:?}"
        )))
    }
}

pub(super) fn assert_action_model(
    action: &Value,
    expected: &Value,
) -> Result<(), ConformanceError> {
    if action.pointer("/params/action/message/model") == Some(expected) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "authoritative chat/turnStarted model was {:?}, expected {expected}",
            action.pointer("/params/action/message/model")
        )))
    }
}

pub(super) fn assert_historical_model(
    state: &Value,
    turn_id: &str,
    expected: &Value,
) -> Result<(), ConformanceError> {
    let model = state
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
                .and_then(|turn| turn.pointer("/message/model"))
        });
    if model == Some(expected) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "completed turn {turn_id:?} retained model {model:?}, expected {expected}"
        )))
    }
}

pub(super) async fn assert_fixture_request(
    stack: &LocalStack,
    index: usize,
    model: &str,
    thinking_level: &str,
) -> Result<(), ConformanceError> {
    for _ in 0..50 {
        if let Some(request) = stack.fixture_requests().get(index) {
            if request.api == ObservedApi::Responses
                && request.model == model
                && request.reasoning_effort.as_deref() == Some(thinking_level)
            {
                return Ok(());
            }
            return Err(ConformanceError::Options(format!(
                "fixture request {index} was {:?}/{:?}/{:?}, expected Responses/{model:?}/{thinking_level:?}",
                request.api, request.model, request.reasoning_effort
            )));
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Err(ConformanceError::Timeout(format!(
        "waiting for fixture request {index}"
    )))
}
