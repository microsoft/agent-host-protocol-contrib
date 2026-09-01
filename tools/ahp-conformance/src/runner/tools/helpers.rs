//! Typed black-box parsing and lifecycle helpers for local tool probes.

use serde_json::Value;

use super::super::advanced::start_turn;
use super::super::{ConformanceError, Probe};
use super::lifecycle::fixture_tool_prompt_with_delay;

pub(in super::super) struct ToolCall {
    pub(in super::super) id: String,
    pub(in super::super) name: String,
}

pub(in super::super) struct ToolReady {
    pub(super) confirmed: Option<String>,
    pub(super) option_ids: Vec<String>,
}

pub(in super::super) struct ToolResult {
    pub(in super::super) success: bool,
    pub(in super::super) text: String,
    pub(in super::super) error: Option<Value>,
}

pub(in super::super) async fn start_fixture_tool_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    tool: &str,
    arguments: Value,
    response: &str,
) -> Result<ToolCall, ConformanceError> {
    start_fixture_tool_turn_with_delay(probe, chat, turn_id, tool, arguments, response, 0).await
}

pub(in super::super) async fn start_fixture_tool_turn_with_delay(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    tool: &str,
    arguments: Value,
    response: &str,
    delay_ms: u64,
) -> Result<ToolCall, ConformanceError> {
    start_turn(
        probe,
        chat,
        turn_id,
        fixture_tool_prompt_with_delay(tool, arguments, response, delay_ms),
    )
    .await?;
    require_turn(
        &probe.wait_action(chat, "chat/turnStarted").await?,
        turn_id,
        "chat/turnStarted",
    )?;
    let start = probe.wait_action(chat, "chat/toolCallStart").await?;
    let call = tool_call(&start, turn_id, "chat/toolCallStart", true)?;
    if call.name != tool {
        return Err(ConformanceError::Options(format!(
            "tool call named {:?}, expected {tool:?}",
            call.name
        )));
    }
    Ok(call)
}

pub(in super::super) async fn await_tool_ready(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    call: &ToolCall,
) -> Result<ToolReady, ConformanceError> {
    let ready = probe.wait_action(chat, "chat/toolCallReady").await?;
    if tool_call(&ready, turn_id, "chat/toolCallReady", false)?.id != call.id {
        return Err(ConformanceError::Options(
            "toolCallReady did not correlate to toolCallStart".to_owned(),
        ));
    }
    let action = action_value(&ready)?;
    Ok(ToolReady {
        confirmed: action
            .get("confirmed")
            .and_then(Value::as_str)
            .map(str::to_owned),
        option_ids: action
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|option| option.get("id").and_then(Value::as_str))
            .map(str::to_owned)
            .collect(),
    })
}

pub(in super::super) async fn await_tool_complete(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    call: &ToolCall,
) -> Result<ToolResult, ConformanceError> {
    let complete = probe.wait_action(chat, "chat/toolCallComplete").await?;
    if tool_call(&complete, turn_id, "chat/toolCallComplete", false)?.id != call.id {
        return Err(ConformanceError::Options(
            "toolCallComplete did not correlate to toolCallStart".to_owned(),
        ));
    }
    parse_tool_result(
        action_value(&complete)?.get("result").ok_or_else(|| {
            ConformanceError::Options("toolCallComplete omitted result".to_owned())
        })?,
    )
}

pub(in super::super) fn require_turn(
    event: &Value,
    turn_id: &str,
    action_type: &str,
) -> Result<(), ConformanceError> {
    if action_value(event)?.get("turnId").and_then(Value::as_str) == Some(turn_id) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "{action_type} did not target turn {turn_id:?}"
        )))
    }
}

pub(in super::super) fn action_value(event: &Value) -> Result<&Value, ConformanceError> {
    event
        .pointer("/params/action")
        .or_else(|| event.get("action"))
        .ok_or_else(|| ConformanceError::Options("action envelope omitted action".to_owned()))
}

pub(in super::super) fn parse_tool_result(value: &Value) -> Result<ToolResult, ConformanceError> {
    let success = value
        .get("success")
        .and_then(Value::as_bool)
        .ok_or_else(|| ConformanceError::Options("tool result omitted success".to_owned()))?;
    let text = value
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ToolResult {
        success,
        text,
        error: value.get("error").cloned().filter(|error| !error.is_null()),
    })
}

fn tool_call(
    event: &Value,
    turn_id: &str,
    action_type: &str,
    require_name: bool,
) -> Result<ToolCall, ConformanceError> {
    require_turn(event, turn_id, action_type)?;
    let action = action_value(event)?;
    Ok(ToolCall {
        id: action
            .get("toolCallId")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| ConformanceError::Options(format!("{action_type} omitted toolCallId")))?
            .to_owned(),
        name: action
            .get("toolName")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .or_else(|| (!require_name).then(String::new))
            .ok_or_else(|| ConformanceError::Options(format!("{action_type} omitted toolName")))?,
    })
}
