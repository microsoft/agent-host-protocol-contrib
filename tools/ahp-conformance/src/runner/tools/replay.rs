//! Replay-specific tool-result and sequence validation.

use serde_json::Value;

use super::super::ConformanceError;
use super::helpers::{parse_tool_result, ToolCall};

pub(super) fn assert_replay_tool_result(
    response: &Value,
    chat: &str,
    turn_id: &str,
    call: &ToolCall,
    expected_success: bool,
) -> Result<(), ConformanceError> {
    let actions = response
        .pointer("/result/actions")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("reconnect omitted replay actions".to_owned()))?;
    assert_strictly_increasing_sequences(actions)?;
    let complete = actions.iter().find(|event| {
        event.get("channel").and_then(Value::as_str) == Some(chat)
            && event.pointer("/action/type").and_then(Value::as_str)
                == Some("chat/toolCallComplete")
            && event.pointer("/action/turnId").and_then(Value::as_str) == Some(turn_id)
            && event.pointer("/action/toolCallId").and_then(Value::as_str) == Some(call.id.as_str())
    });
    let result = complete
        .and_then(|event| event.pointer("/action/result"))
        .ok_or_else(|| {
            ConformanceError::Options("reconnect omitted completed tool result".to_owned())
        })?;
    let result = parse_tool_result(result)?;
    if result.success == expected_success && !result.text.is_empty() {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "replayed tool result lost its success value or content".to_owned(),
        ))
    }
}

pub(super) fn assert_strictly_increasing_sequences(
    actions: &[Value],
) -> Result<(), ConformanceError> {
    let mut previous = -1;
    for action in actions {
        let sequence = action
            .get("serverSeq")
            .and_then(Value::as_i64)
            .ok_or_else(|| ConformanceError::Options("action omitted serverSeq".to_owned()))?;
        if sequence <= previous {
            return Err(ConformanceError::Options(
                "serverSeq values are not strictly increasing".to_owned(),
            ));
        }
        previous = sequence;
    }
    Ok(())
}
