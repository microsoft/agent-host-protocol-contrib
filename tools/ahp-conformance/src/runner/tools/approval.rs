//! Confirmation dispatch and authoritative-confirmation parsing.

use serde_json::{json, Value};

use super::super::{ConformanceError, Probe};
use super::helpers::{action_value, require_turn, ToolCall};

pub(super) struct Confirmation {
    pub(super) approved: bool,
    pub(super) confirmed: Option<String>,
    pub(super) selected_option_id: String,
}

pub(super) async fn confirm(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    call: &ToolCall,
    option: &str,
) -> Result<(), ConformanceError> {
    probe
        .dispatch_action(
            chat,
            json!({
                "type": "chat/toolCallConfirmed",
                "turnId": turn_id,
                "toolCallId": &call.id,
                "approved": option != "deny",
                "selectedOptionId": option,
            }),
        )
        .await
}

pub(super) async fn await_confirmation(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    call: &ToolCall,
) -> Result<Confirmation, ConformanceError> {
    let event = probe.wait_action(chat, "chat/toolCallConfirmed").await?;
    let action = action_value(&event)?;
    require_turn(&event, turn_id, "chat/toolCallConfirmed")?;
    if action.get("toolCallId").and_then(Value::as_str) != Some(call.id.as_str()) {
        return Err(ConformanceError::Options(
            "toolCallConfirmed did not target the pending tool call".to_owned(),
        ));
    }
    Ok(Confirmation {
        approved: action
            .get("approved")
            .and_then(Value::as_bool)
            .ok_or_else(|| ConformanceError::Options("confirmation omitted approved".to_owned()))?,
        confirmed: action
            .get("confirmed")
            .and_then(Value::as_str)
            .map(str::to_owned),
        selected_option_id: action
            .get("selectedOptionId")
            .and_then(Value::as_str)
            .ok_or_else(|| ConformanceError::Options("confirmation omitted option ID".to_owned()))?
            .to_owned(),
    })
}

pub(super) fn assert_confirmation(
    confirmation: &Confirmation,
    approved: bool,
    option: &str,
    requires_confirmation: bool,
) -> Result<(), ConformanceError> {
    if confirmation.approved == approved
        && confirmation.selected_option_id == option
        && (!requires_confirmation || confirmation.confirmed.is_some())
        && (approved || confirmation.confirmed.is_none())
    {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "server did not publish the authoritative requested shell confirmation".to_owned(),
        ))
    }
}

pub(super) fn is_setting(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        value
            .replace(['-', '_'], "")
            .eq_ignore_ascii_case("setting")
    })
}
