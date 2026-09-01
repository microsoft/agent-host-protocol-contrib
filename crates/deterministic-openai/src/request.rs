//! Shared request state and validation helpers for fixture endpoint parsers.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::contract::{code, ContractError};

pub(crate) struct FixtureRequest {
    pub(crate) last_user_content: String,
    pub(crate) last_user_index: Option<usize>,
    pub(crate) messages: Vec<RequestMessage>,
    pub(crate) stream: bool,
    pub(crate) tool_names: HashSet<String>,
}

pub(crate) enum RequestMessage {
    User(String),
    Tool(ToolResult),
    Other,
}

pub(crate) struct ToolResult {
    pub(crate) tool_call_id: String,
}

impl FixtureRequest {
    pub(crate) fn parse_chat(value: Value) -> Result<Self, ContractError> {
        super::request_chat::parse(value)
    }

    pub(crate) fn parse_responses(value: Value) -> Result<Self, ContractError> {
        super::request_responses::parse(value)
    }

    pub(super) fn finish(
        object: &Map<String, Value>,
        messages: Vec<RequestMessage>,
        tool_names: HashSet<String>,
    ) -> Result<Self, ContractError> {
        if messages.is_empty() {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                "messages or input must be a non-empty array",
            ));
        }
        let stream = match object.get("stream") {
            Some(Value::Bool(stream)) => *stream,
            Some(_) => {
                return Err(ContractError::new(
                    code::INVALID_REQUEST,
                    "stream must be a boolean when supplied",
                ))
            }
            None => false,
        };
        let (last_user_index, last_user_content) = messages
            .iter()
            .enumerate()
            .filter_map(|(index, message)| match message {
                RequestMessage::User(content) => Some((index, content.clone())),
                _ => None,
            })
            .next_back()
            .map_or((None, String::new()), |(index, content)| {
                (Some(index), content)
            });
        Ok(Self {
            last_user_content,
            last_user_index,
            messages,
            stream,
            tool_names,
        })
    }

    pub(crate) fn tool_results_after_last_user(&self) -> impl Iterator<Item = &ToolResult> {
        let start = self.last_user_index.map_or(0, |index| index + 1);
        self.messages[start..]
            .iter()
            .filter_map(|message| match message {
                RequestMessage::Tool(result) => Some(result),
                _ => None,
            })
    }
}

pub(super) fn request_object(value: &Value) -> Result<&Map<String, Value>, ContractError> {
    let object = value.as_object().ok_or_else(|| {
        ContractError::new(code::INVALID_REQUEST, "request body must be a JSON object")
    })?;
    let model = required_string(object, "model", "model")?;
    if model.trim().is_empty() {
        return Err(ContractError::new(
            code::INVALID_REQUEST,
            "model must be a non-empty string",
        ));
    }
    Ok(object)
}

pub(super) fn parse_tools<F>(
    value: Option<&Value>,
    name: F,
) -> Result<HashSet<String>, ContractError>
where
    F: Fn(&Map<String, Value>, &str) -> Result<String, ContractError>,
{
    let Some(value) = value else {
        return Ok(HashSet::new());
    };
    let tools = value.as_array().ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            "tools must be an array when supplied",
        )
    })?;
    let mut names = HashSet::new();
    for (index, tool) in tools.iter().enumerate() {
        let path = format!("tools[{index}]");
        let object = tool.as_object().ok_or_else(|| {
            ContractError::new(code::INVALID_REQUEST, format!("{path} must be an object"))
        })?;
        if required_string(object, "type", &format!("{path}.type"))? != "function" {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                format!("{path}.type must be \"function\""),
            ));
        }
        if !names.insert(name(object, &path)?) {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                format!("{path} duplicates a function name"),
            ));
        }
    }
    Ok(names)
}

pub(super) fn item_object<'a>(
    index: usize,
    value: &'a Value,
    collection: &str,
) -> Result<&'a Map<String, Value>, ContractError> {
    value.as_object().ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            format!("{collection}[{index}] must be an object"),
        )
    })
}

pub(super) fn required_array<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Vec<Value>, ContractError> {
    object.get(key).and_then(Value::as_array).ok_or_else(|| {
        ContractError::new(code::INVALID_REQUEST, format!("{path} must be an array"))
    })
}

pub(super) fn required_object<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a Map<String, Value>, ContractError> {
    object.get(key).and_then(Value::as_object).ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            format!("{path}.{key} must be an object"),
        )
    })
}

pub(super) fn required_string<'a>(
    object: &'a Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<&'a str, ContractError> {
    object.get(key).and_then(Value::as_str).ok_or_else(|| {
        ContractError::new(code::INVALID_REQUEST, format!("{path} must be a string"))
    })
}

pub(super) fn non_empty_string(
    object: &Map<String, Value>,
    key: &str,
    path: &str,
) -> Result<String, ContractError> {
    let value = required_string(object, key, path)?;
    (!value.trim().is_empty())
        .then(|| value.to_owned())
        .ok_or_else(|| {
            ContractError::new(code::INVALID_REQUEST, format!("{path} must not be empty"))
        })
}
