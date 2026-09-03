//! OpenAI request DTO serialization for the supported endpoint protocols.

use serde_json::{json, Map, Value};

use crate::{ModelApi, ModelError, ModelMessage, ModelRequest, ModelToolCall, ModelToolDefinition};

pub(crate) fn request_json(
    api: ModelApi,
    configured_model: Option<&str>,
    request: &ModelRequest,
) -> Result<Value, ModelError> {
    let (model, reasoning_effort) = request.selected_model(configured_model)?;
    match api {
        ModelApi::Responses | ModelApi::Auto => responses_json(model, reasoning_effort, request),
        ModelApi::ChatCompletions => Ok(chat_completions_json(model, reasoning_effort, request)),
    }
}

fn chat_completions_json(
    model: &str,
    reasoning_effort: Option<crate::ReasoningEffort>,
    request: &ModelRequest,
) -> Value {
    let mut object = Map::from_iter([
        ("model".to_owned(), json!(model)),
        (
            "messages".to_owned(),
            Value::Array(request.messages.iter().map(chat_message_json).collect()),
        ),
        (
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(chat_tool_json).collect()),
        ),
        ("parallel_tool_calls".to_owned(), json!(false)),
        ("stream".to_owned(), json!(true)),
    ]);
    if let Some(reasoning_effort) = reasoning_effort {
        object.insert(
            "reasoning_effort".to_owned(),
            json!(reasoning_effort.as_str()),
        );
    }
    Value::Object(object)
}

fn responses_json(
    model: &str,
    reasoning_effort: Option<crate::ReasoningEffort>,
    request: &ModelRequest,
) -> Result<Value, ModelError> {
    let mut input = Vec::new();
    let messages = match &request.response_continuation {
        Some(continuation) => {
            if continuation.previous_response_id.trim().is_empty() {
                return Err(ModelError::InvalidResponse(
                    "Responses continuation id must not be empty".to_owned(),
                ));
            }
            request
                .messages
                .get(continuation.input_start..)
                .ok_or_else(|| {
                    ModelError::InvalidResponse(
                        "Responses continuation input_start exceeded the message count".to_owned(),
                    )
                })?
        }
        None => request.messages.as_slice(),
    };
    for message in messages {
        match message {
            ModelMessage::User { content } => {
                input.push(json!({ "role": "user", "content": content }));
            }
            ModelMessage::Assistant {
                content,
                tool_calls,
            } => {
                if let Some(content) = content {
                    input.push(json!({ "role": "assistant", "content": content }));
                }
                input.extend(tool_calls.iter().map(responses_tool_call_json));
            }
            ModelMessage::Tool {
                tool_call_id,
                content,
            } => {
                input.push(json!({
                    "type": "function_call_output",
                    "call_id": tool_call_id,
                    "output": content,
                }));
            }
        }
    }
    let mut object = Map::from_iter([
        ("model".to_owned(), json!(model)),
        ("input".to_owned(), Value::Array(input)),
        (
            "tools".to_owned(),
            Value::Array(request.tools.iter().map(responses_tool_json).collect()),
        ),
        ("parallel_tool_calls".to_owned(), json!(false)),
        ("stream".to_owned(), json!(true)),
    ]);
    if let Some(reasoning_effort) = reasoning_effort {
        object.insert(
            "reasoning".to_owned(),
            json!({ "effort": reasoning_effort.as_str() }),
        );
    }
    if let Some(continuation) = &request.response_continuation {
        object.insert(
            "previous_response_id".to_owned(),
            json!(continuation.previous_response_id),
        );
    }
    if request.store_response {
        object.insert("store".to_owned(), Value::Bool(true));
    }
    Ok(Value::Object(object))
}

fn chat_message_json(message: &ModelMessage) -> Value {
    match message {
        ModelMessage::User { content } => json!({ "role": "user", "content": content }),
        ModelMessage::Assistant {
            content,
            tool_calls,
        } => {
            let mut object = Map::from_iter([
                ("role".to_owned(), json!("assistant")),
                ("content".to_owned(), json!(content)),
            ]);
            if !tool_calls.is_empty() {
                object.insert(
                    "tool_calls".to_owned(),
                    Value::Array(tool_calls.iter().map(chat_tool_call_json).collect()),
                );
            }
            Value::Object(object)
        }
        ModelMessage::Tool {
            tool_call_id,
            content,
        } => json!({ "role": "tool", "tool_call_id": tool_call_id, "content": content }),
    }
}

fn chat_tool_json(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.input_schema,
        },
    })
}

fn responses_tool_json(tool: &ModelToolDefinition) -> Value {
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    })
}

fn chat_tool_call_json(call: &ModelToolCall) -> Value {
    json!({
        "id": call.id,
        "type": "function",
        "function": {"name": call.name, "arguments": call.arguments},
    })
}

fn responses_tool_call_json(call: &ModelToolCall) -> Value {
    json!({
        "type": "function_call",
        "call_id": call.id,
        "name": call.name,
        "arguments": call.arguments,
    })
}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
