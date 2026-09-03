use std::time::Duration;

use ahp_types::actions::{ActionEnvelope, ChatTurnStartedAction};
use ahp_types::state::Message;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::{ConformanceError, Probe};

pub(super) const REASONING_MODEL: &str = "fixture-reasoning";

pub(super) struct SessionChat {
    pub(super) session: String,
    pub(super) chat: String,
}

pub(super) async fn initialized_probe(
    target: &str,
    timeout: Duration,
    name: &str,
) -> Result<Probe, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let initialized = probe
        .initialize(
            &format!("{name}-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    require_result(&initialized, "initialize")?;
    Ok(probe)
}

pub(super) async fn create_session_without_config(
    probe: &mut Probe,
    suffix: &str,
) -> Result<SessionChat, ConformanceError> {
    let session = format!("ahp-session:/{}-{}", suffix, Uuid::new_v4());
    let created = probe
        .request(
            "createSession",
            json!({ "channel": &session, "provider": "openai" }),
        )
        .await?;
    require_result(&created, "createSession without config")?;
    let state = session_state(probe, &session).await?;
    let chat = state
        .get("defaultChat")
        .and_then(Value::as_str)
        .or_else(|| state.pointer("/chats/0/resource").and_then(Value::as_str))
        .filter(|chat| chat.starts_with("ahp-chat:/"))
        .ok_or_else(|| ConformanceError::Options("session has no default chat".to_owned()))?
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
    state_from_reply(&subscribed, "session subscribe")
}

pub(super) async fn chat_state(probe: &mut Probe, chat: &str) -> Result<Value, ConformanceError> {
    let subscribed = probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    state_from_reply(&subscribed, "chat subscribe")
}

pub(super) fn user_draft(text: &str, model: Option<Value>) -> Result<Value, ConformanceError> {
    let mut draft = json!({
        "text": text,
        "origin": { "kind": "user" },
    });
    if let Some(model) = model {
        draft
            .as_object_mut()
            .ok_or_else(|| ConformanceError::Options("draft was not an object".to_owned()))?
            .insert("model".to_owned(), model);
    }
    let _: Message = serde_json::from_value(draft.clone())?;
    Ok(draft)
}

pub(super) fn conditional_draft(
    mut draft: Value,
    expected_turn_id: Option<&str>,
) -> Result<Value, ConformanceError> {
    draft
        .as_object_mut()
        .ok_or_else(|| ConformanceError::Options("conditional draft was not an object".to_owned()))?
        .insert(
            "_meta".to_owned(),
            json!({
                "com.microsoft.agent-host-protocol-samples.expectedTurnId": expected_turn_id
            }),
        );
    let _: Message = serde_json::from_value(draft.clone())?;
    Ok(draft)
}

pub(super) fn reasoning_selection(level: &str) -> Value {
    json!({
        "id": REASONING_MODEL,
        "config": { "thinkingLevel": level },
    })
}

pub(super) async fn send_draft(
    probe: &mut Probe,
    chat: &str,
    draft: Option<Value>,
) -> Result<i64, ConformanceError> {
    let mut action = json!({ "type": "chat/draftChanged" });
    if let Some(draft) = draft {
        action
            .as_object_mut()
            .ok_or_else(|| ConformanceError::Options("draft action was not an object".to_owned()))?
            .insert("draft".to_owned(), draft);
    }
    probe.dispatch_action_with_sequence(chat, action).await
}

pub(super) async fn send_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    message: Value,
) -> Result<i64, ConformanceError> {
    let action = json!({
        "type": "chat/turnStarted",
        "turnId": turn_id,
        "startedAt": "2026-09-02T00:00:00.000Z",
        "message": message,
    });
    let _: ChatTurnStartedAction = serde_json::from_value(action.clone())?;
    probe.dispatch_action_with_sequence(chat, action).await
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

pub(super) fn assert_no_default_session_config(reply: &Value) -> Result<(), ConformanceError> {
    if require_result(reply, "model-less session config")?.get("values") == Some(&json!({})) {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "model-less fixture unexpectedly resolved a catalog default".to_owned(),
        ))
    }
}

pub(super) fn assert_action_draft(
    event: &Value,
    expected: Option<&Value>,
    client_seq: i64,
) -> Result<(), ConformanceError> {
    let _: ActionEnvelope = serde_json::from_value(
        event
            .get("params")
            .cloned()
            .ok_or_else(|| ConformanceError::Options("action omitted params".to_owned()))?,
    )?;
    if event
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
        || event
            .pointer("/params/origin/clientSeq")
            .and_then(Value::as_i64)
            != Some(client_seq)
        || event.pointer("/params/action/draft") != expected
    {
        return Err(ConformanceError::Options(
            "chat/draftChanged was not the expected authoritative action".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn assert_rejected(
    event: &Value,
    expected_type: &str,
    client_seq: i64,
) -> Result<(), ConformanceError> {
    if event.pointer("/params/action/type").and_then(Value::as_str) != Some(expected_type)
        || event
            .pointer("/params/origin/clientSeq")
            .and_then(Value::as_i64)
            != Some(client_seq)
        || event
            .pointer("/params/rejectionReason")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(ConformanceError::Options(format!(
            "invalid {expected_type} was not authoritatively rejected"
        )));
    }
    Ok(())
}

pub(super) fn assert_snapshot_draft(
    state: &Value,
    expected: &Value,
) -> Result<(), ConformanceError> {
    if state.get("draft") == Some(expected) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "chat snapshot draft was {:?}, expected {expected}",
            state.get("draft")
        )))
    }
}

pub(super) fn assert_snapshot_without_draft(state: &Value) -> Result<(), ConformanceError> {
    if state.get("draft").is_none() {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "chat snapshot retained a cleared draft instead of omitting it".to_owned(),
        ))
    }
}

fn state_from_reply(reply: &Value, operation: &str) -> Result<Value, ConformanceError> {
    require_result(reply, operation)?
        .pointer("/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options(format!("{operation} omitted snapshot state")))
}
