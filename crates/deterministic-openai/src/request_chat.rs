//! Chat Completions request parser for the deterministic fixture.

use std::collections::HashSet;

use serde_json::Value;

use super::contract::{code, ContractError};
use super::request::{
    item_object, non_empty_string, optional_reasoning_effort, parse_tools, request_object,
    required_array, required_object, required_string, FixtureRequest, RequestMessage, ToolResult,
};

pub(super) fn parse(value: Value) -> Result<FixtureRequest, ContractError> {
    let object = request_object(&value)?;
    let messages = required_array(object, "messages", "messages")?;
    let mut call_ids = HashSet::new();
    let messages = messages
        .iter()
        .enumerate()
        .map(|(index, value)| parse_message(index, value, &mut call_ids))
        .collect::<Result<Vec<_>, _>>()?;
    FixtureRequest::finish(
        object,
        messages,
        parse_tools(object.get("tools"), tool_name)?,
        optional_reasoning_effort(object.get("reasoning_effort"), "reasoning_effort")?,
    )
}

fn parse_message(
    index: usize,
    value: &Value,
    call_ids: &mut HashSet<String>,
) -> Result<RequestMessage, ContractError> {
    let object = item_object(index, value, "messages")?;
    match required_string(object, "role", &format!("messages[{index}].role"))? {
        "user" => {
            reject_role_field(object, index, "tool_calls")?;
            reject_role_field(object, index, "tool_call_id")?;
            Ok(RequestMessage::User(
                required_string(object, "content", &format!("messages[{index}].content"))?
                    .to_owned(),
            ))
        }
        "assistant" => {
            reject_role_field(object, index, "tool_call_id")?;
            if !matches!(object.get("content"), Some(Value::String(_) | Value::Null)) {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    format!("messages[{index}].content must be a string or null for assistant"),
                ));
            }
            if let Some(tool_calls) = object.get("tool_calls") {
                for (call_index, call) in tool_calls
                    .as_array()
                    .ok_or_else(|| {
                        ContractError::new(
                            code::INVALID_REQUEST,
                            format!("messages[{index}].tool_calls must be an array"),
                        )
                    })?
                    .iter()
                    .enumerate()
                {
                    parse_tool_call(index, call_index, call, call_ids)?;
                }
            }
            Ok(object
                .get("content")
                .and_then(Value::as_str)
                .map_or(RequestMessage::Other, |content| {
                    RequestMessage::Assistant(content.to_owned())
                }))
        }
        "tool" => {
            reject_role_field(object, index, "tool_calls")?;
            required_string(object, "content", &format!("messages[{index}].content"))?;
            Ok(RequestMessage::Tool(ToolResult {
                tool_call_id: non_empty_string(
                    object,
                    "tool_call_id",
                    &format!("messages[{index}].tool_call_id"),
                )?,
            }))
        }
        _ => Ok(RequestMessage::Other),
    }
}

fn parse_tool_call(
    message_index: usize,
    call_index: usize,
    value: &Value,
    call_ids: &mut HashSet<String>,
) -> Result<(), ContractError> {
    let path = format!("messages[{message_index}].tool_calls[{call_index}]");
    let object = value.as_object().ok_or_else(|| {
        ContractError::new(code::INVALID_REQUEST, format!("{path} must be an object"))
    })?;
    let id = non_empty_string(object, "id", &format!("{path}.id"))?;
    if !call_ids.insert(id) {
        return Err(ContractError::new(
            code::INVALID_REQUEST,
            format!("{path}.id duplicates an earlier assistant tool call id"),
        ));
    }
    if required_string(object, "type", &format!("{path}.type"))? != "function" {
        return Err(ContractError::new(
            code::INVALID_REQUEST,
            format!("{path}.type must be \"function\""),
        ));
    }
    let function = required_object(object, "function", &path)?;
    non_empty_string(function, "name", &format!("{path}.function.name"))?;
    required_string(function, "arguments", &format!("{path}.function.arguments"))?;
    Ok(())
}

fn tool_name(object: &serde_json::Map<String, Value>, path: &str) -> Result<String, ContractError> {
    let function = required_object(object, "function", path)?;
    non_empty_string(function, "name", &format!("{path}.function.name"))
}

fn reject_role_field(
    object: &serde_json::Map<String, Value>,
    index: usize,
    field: &str,
) -> Result<(), ContractError> {
    (!object.contains_key(field)).then_some(()).ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            format!("messages[{index}].{field} is not supported for this role"),
        )
    })
}
