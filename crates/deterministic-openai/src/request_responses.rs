//! Responses API request parser for the deterministic fixture.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::contract::{code, ContractError};
use super::request::{
    item_object, non_empty_string, parse_tools, reasoning_effort, request_object, required_array,
    required_string, FixtureRequest, RequestMessage, ToolResult,
};

pub(super) fn parse(value: Value) -> Result<FixtureRequest, ContractError> {
    let object = request_object(&value)?;
    let input = required_array(object, "input", "input")?;
    let mut pending_call_ids = HashSet::new();
    let messages = input
        .iter()
        .enumerate()
        .map(|(index, value)| parse_item(index, value, &mut pending_call_ids))
        .collect::<Result<Vec<_>, _>>()?;
    FixtureRequest::finish(
        object,
        messages,
        parse_tools(object.get("tools"), tool_name)?,
        parse_reasoning_effort(object)?,
    )
}

fn parse_reasoning_effort(object: &Map<String, Value>) -> Result<Option<String>, ContractError> {
    let Some(reasoning) = object.get("reasoning") else {
        return Ok(None);
    };
    let reasoning = reasoning.as_object().ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            "reasoning must be an object when supplied",
        )
    })?;
    let effort = reasoning.get("effort").ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            "reasoning.effort is required when reasoning is supplied",
        )
    })?;
    reasoning_effort(effort, "reasoning.effort").map(Some)
}

fn parse_item(
    index: usize,
    value: &Value,
    call_ids: &mut HashSet<String>,
) -> Result<RequestMessage, ContractError> {
    let object = item_object(index, value, "input")?;
    match object.get("type").and_then(Value::as_str) {
        Some("function_call") => {
            let id = non_empty_string(object, "call_id", &format!("input[{index}].call_id"))?;
            if !call_ids.insert(id) {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    format!("input[{index}].call_id duplicates an earlier function call"),
                ));
            }
            non_empty_string(object, "name", &format!("input[{index}].name"))?;
            required_string(object, "arguments", &format!("input[{index}].arguments"))?;
            Ok(RequestMessage::Other)
        }
        Some("function_call_output") => {
            let id = non_empty_string(object, "call_id", &format!("input[{index}].call_id"))?;
            if !call_ids.remove(&id) {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    format!("input[{index}].call_id has no preceding pending function call"),
                ));
            }
            required_string(object, "output", &format!("input[{index}].output"))?;
            Ok(RequestMessage::Tool(ToolResult { tool_call_id: id }))
        }
        Some(_) => Ok(RequestMessage::Other),
        None => match required_string(object, "role", &format!("input[{index}].role"))? {
            "user" => Ok(RequestMessage::User(
                required_string(object, "content", &format!("input[{index}].content"))?.to_owned(),
            )),
            "assistant" => {
                required_string(object, "content", &format!("input[{index}].content"))?;
                Ok(RequestMessage::Other)
            }
            _ => Ok(RequestMessage::Other),
        },
    }
}

fn tool_name(object: &Map<String, Value>, path: &str) -> Result<String, ContractError> {
    non_empty_string(object, "name", &format!("{path}.name"))
}
