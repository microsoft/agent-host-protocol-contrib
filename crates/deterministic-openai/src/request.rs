//! Shared request state and validation helpers for fixture endpoint parsers.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::contract::{code, ContractError};

pub(crate) struct FixtureRequest {
    pub(crate) model: String,
    pub(crate) reasoning_effort: Option<String>,
    pub(crate) last_user_content: String,
    pub(crate) last_user_index: Option<usize>,
    pub(crate) messages: Vec<RequestMessage>,
    pub(crate) stream: bool,
    pub(crate) tool_names: HashSet<String>,
    pub(crate) tools: Vec<ToolDefinition>,
}

pub(crate) enum RequestMessage {
    User(String),
    Assistant(String),
    Tool(ToolResult),
    Context(String),
    Other,
}

pub(crate) struct ToolResult {
    pub(crate) tool_call_id: String,
    pub(crate) content: String,
}

pub(crate) struct ToolDefinition {
    pub(crate) name: String,
    pub(crate) parameters: Value,
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
        tools: Vec<ToolDefinition>,
        reasoning_effort: Option<String>,
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
            model: non_empty_string(object, "model", "model")?,
            reasoning_effort,
            last_user_content,
            last_user_index,
            messages,
            stream,
            tool_names: tools.iter().map(|tool| tool.name.clone()).collect(),
            tools,
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

    pub(crate) fn previous_assistant_content(&self) -> Option<&str> {
        let end = self.last_user_index?;
        self.messages[..end]
            .iter()
            .rev()
            .find_map(|message| match message {
                RequestMessage::Assistant(content) => Some(content.as_str()),
                _ => None,
            })
    }
}

pub(super) fn optional_reasoning_effort(
    value: Option<&Value>,
    path: &str,
) -> Result<Option<String>, ContractError> {
    value.map(|value| reasoning_effort(value, path)).transpose()
}

pub(super) fn reasoning_effort(value: &Value, path: &str) -> Result<String, ContractError> {
    let effort = value.as_str().ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            format!("{path} must be a supported reasoning-effort string"),
        )
    })?;
    matches!(
        effort,
        "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
    )
    .then(|| effort.to_owned())
    .ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            format!("{path} must be one of none, minimal, low, medium, high, xhigh, or max"),
        )
    })
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
) -> Result<Vec<ToolDefinition>, ContractError>
where
    F: Fn(&Map<String, Value>, &str) -> Result<ToolDefinition, ContractError>,
{
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let tools = value.as_array().ok_or_else(|| {
        ContractError::new(
            code::INVALID_REQUEST,
            "tools must be an array when supplied",
        )
    })?;
    let mut names = HashSet::new();
    let mut definitions = Vec::new();
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
        let definition = name(object, &path)?;
        if !names.insert(definition.name.clone()) {
            return Err(ContractError::new(
                code::INVALID_REQUEST,
                format!("{path} duplicates a function name"),
            ));
        }
        definitions.push(definition);
    }
    Ok(definitions)
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
