use std::time::Duration;

use deterministic_openai::ObservedApi;
use serde_json::{json, Value};

use super::super::local_stack::LocalStack;
use super::super::{ConformanceError, Probe};

pub(super) async fn dispatch_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    prompt: String,
    model: Option<Value>,
) -> Result<(), ConformanceError> {
    let mut message = json!({ "text": prompt, "origin": { "kind": "user" } });
    if let Some(model) = model {
        let message = message
            .as_object_mut()
            .ok_or_else(|| ConformanceError::Options("turn message was not an object".into()))?;
        message.insert("model".into(), model);
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
