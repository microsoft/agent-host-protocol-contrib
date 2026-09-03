use serde_json::{json, Value};
use uuid::Uuid;

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
        || error
            .get("message")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
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
            .get("properties")
            .and_then(|properties| properties.get("id"))
            .and_then(|id| id.get("enum"))
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty)
        || !schema_supports_expected_thinking(model, expected)
    {
        return Err(ConformanceError::Options(
            "session config schema did not expose a mutable model selection".into(),
        ));
    }
    Ok(())
}

fn schema_supports_expected_thinking(
    model: &serde_json::Map<String, Value>,
    expected: &Value,
) -> bool {
    let thinking = model
        .get("properties")
        .and_then(|properties| properties.get("config"))
        .and_then(|config| config.get("properties"))
        .and_then(|properties| properties.get("thinkingLevel"))
        .and_then(|thinking| thinking.get("enum"))
        .and_then(Value::as_array);
    match expected
        .pointer("/config/thinkingLevel")
        .and_then(Value::as_str)
    {
        Some(expected) => thinking
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(expected))),
        None => thinking.is_some(),
    }
}
