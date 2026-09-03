use ahp_types::actions::ActionEnvelope;
use serde_json::Value;

use super::super::{ConformanceError, Probe};
use super::ordering_text::assert_final_text;

pub(super) const FINAL_RESPONSE: &str = "final text after the tool";
pub(super) const TURN_ID: &str = "response-part-ordering";
const ACTION_TYPES: &[&str] = &[
    "chat/turnStarted",
    "chat/responsePart",
    "chat/toolCallStart",
    "chat/toolCallDelta",
    "chat/toolCallReady",
    "chat/toolCallComplete",
    "chat/delta",
    "chat/turnComplete",
    "chat/error",
];

#[derive(Clone, Debug, PartialEq)]
pub(super) struct OrderedAction {
    pub(super) server_seq: i64,
    pub(super) action: Value,
}

pub(super) async fn collect_turn_actions(
    probe: &mut Probe,
    chat: &str,
) -> Result<Vec<OrderedAction>, ConformanceError> {
    let mut actions = Vec::new();
    loop {
        let event = probe.wait_action_any(chat, ACTION_TYPES).await?;
        let action = event.pointer("/params/action").cloned().ok_or_else(|| {
            ConformanceError::Options("chat action omitted its payload".to_owned())
        })?;
        if action.get("turnId").and_then(Value::as_str) != Some(TURN_ID) {
            return Err(ConformanceError::Options(
                "response ordering received an action for another turn".to_owned(),
            ));
        }
        let server_seq = event
            .pointer("/params/serverSeq")
            .and_then(Value::as_i64)
            .ok_or_else(|| ConformanceError::Options("chat action omitted serverSeq".to_owned()))?;
        let terminal = action
            .get("type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        actions.push(OrderedAction { server_seq, action });
        match terminal.as_deref() {
            Some("chat/turnComplete") => return Ok(actions),
            Some("chat/error") => {
                return Err(ConformanceError::Options(
                    "response ordering turn ended in chat/error".to_owned(),
                ))
            }
            _ => {}
        }
    }
}

pub(super) fn replay_actions(
    reply: &Value,
    chat: &str,
) -> Result<Vec<OrderedAction>, ConformanceError> {
    let actions = super::helpers::require_result(reply, "response ordering reconnect")?
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ConformanceError::Options("response ordering reconnect omitted actions".into())
        })?;
    actions
        .iter()
        .filter(|envelope| {
            envelope.get("channel").and_then(Value::as_str) == Some(chat)
                && envelope.pointer("/action/turnId").and_then(Value::as_str) == Some(TURN_ID)
        })
        .map(|envelope| {
            let _: ActionEnvelope = serde_json::from_value(envelope.clone())?;
            Ok(OrderedAction {
                server_seq: envelope
                    .get("serverSeq")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| {
                        ConformanceError::Options("replayed action omitted serverSeq".to_owned())
                    })?,
                action: envelope.get("action").cloned().ok_or_else(|| {
                    ConformanceError::Options("replayed action omitted payload".to_owned())
                })?,
            })
        })
        .collect()
}

pub(super) fn assert_ordered_lifecycle(actions: &[OrderedAction]) -> Result<(), ConformanceError> {
    assert_strict_sequences(actions)?;
    let started = exactly_one(actions, "chat/turnStarted")?;
    let first_part = response_part(actions, 0)?;
    let final_part = response_part(actions, 1)?;
    let tool_start = exactly_one(actions, "chat/toolCallStart")?;
    let tool_ready = exactly_one(actions, "chat/toolCallReady")?;
    let tool_complete = exactly_one(actions, "chat/toolCallComplete")?;
    let complete = exactly_one(actions, "chat/turnComplete")?;
    let initial_id = markdown_part_id(&actions[first_part].action)?;
    let final_id = markdown_part_id(&actions[final_part].action)?;
    if initial_id == final_id
        || !(started < first_part
            && first_part < tool_start
            && tool_start < tool_ready
            && tool_ready < tool_complete
            && tool_complete < final_part
            && final_part < complete)
    {
        return Err(ConformanceError::Options(
            "response-part, tool, and terminal actions were not in the required order".to_owned(),
        ));
    }
    assert_tool_lifecycle(actions, tool_start, tool_ready, tool_complete)?;
    assert_final_text(
        actions,
        first_part,
        final_part,
        complete,
        &initial_id,
        &final_id,
    )
}

pub(super) fn same_actions(live: &[OrderedAction], replayed: &[OrderedAction]) -> bool {
    live == replayed
}

fn assert_tool_lifecycle(
    actions: &[OrderedAction],
    start: usize,
    ready: usize,
    complete: usize,
) -> Result<(), ConformanceError> {
    let tool_id = actions[start]
        .action
        .get("toolCallId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ConformanceError::Options("toolCallStart omitted toolCallId".to_owned()))?;
    let deltas = actions
        .iter()
        .enumerate()
        .filter(|(_, action)| action_type(action) == Some("chat/toolCallDelta"))
        .collect::<Vec<_>>();
    if deltas.is_empty()
        || deltas.iter().any(|(index, action)| {
            *index <= start
                || *index >= ready
                || action.action.get("toolCallId").and_then(Value::as_str) != Some(tool_id)
        })
        || [ready, complete].iter().any(|index| {
            actions[*index]
                .action
                .get("toolCallId")
                .and_then(Value::as_str)
                != Some(tool_id)
        })
    {
        return Err(ConformanceError::Options(
            "tool call deltas, ready, and complete did not form one ordered lifecycle".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn response_part(
    actions: &[OrderedAction],
    nth: usize,
) -> Result<usize, ConformanceError> {
    let parts = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            (action_type(action) == Some("chat/responsePart")).then_some(index)
        })
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        return Err(ConformanceError::Options(format!(
            "tool-to-text turn created {} response parts instead of two",
            parts.len()
        )));
    }
    Ok(parts[nth])
}

fn exactly_one(actions: &[OrderedAction], kind: &str) -> Result<usize, ConformanceError> {
    let matching = actions
        .iter()
        .enumerate()
        .filter_map(|(index, action)| (action_type(action) == Some(kind)).then_some(index))
        .collect::<Vec<_>>();
    if matching.len() == 1 {
        Ok(matching[0])
    } else {
        Err(ConformanceError::Options(format!(
            "expected exactly one {kind}, found {}",
            matching.len()
        )))
    }
}

pub(super) fn markdown_part_id(action: &Value) -> Result<String, ConformanceError> {
    if action.pointer("/part/kind").and_then(Value::as_str) != Some("markdown") {
        return Err(ConformanceError::Options(
            "chat/responsePart was not a markdown response part".to_owned(),
        ));
    }
    action
        .pointer("/part/id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            ConformanceError::Options("markdown response part omitted its id".to_owned())
        })
}

pub(super) fn action_type(action: &OrderedAction) -> Option<&str> {
    action.action.get("type").and_then(Value::as_str)
}

fn assert_strict_sequences(actions: &[OrderedAction]) -> Result<(), ConformanceError> {
    if actions
        .windows(2)
        .any(|pair| pair[0].server_seq >= pair[1].server_seq)
    {
        Err(ConformanceError::Options(
            "response ordering actions did not have strictly increasing serverSeq values"
                .to_owned(),
        ))
    } else {
        Ok(())
    }
}
