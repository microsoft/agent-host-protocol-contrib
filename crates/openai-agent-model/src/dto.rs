//! OpenAI request DTO serialization for the supported endpoint protocols.

use serde_json::{json, Map, Value};

use crate::{ModelApi, ModelMessage, ModelRequest, ModelToolCall, ModelToolDefinition};

pub(crate) fn request_json(api: ModelApi, model: &str, request: &ModelRequest) -> Value {
    match api {
        ModelApi::Responses | ModelApi::Auto => responses_json(model, request),
        ModelApi::ChatCompletions => chat_completions_json(model, request),
    }
}

fn chat_completions_json(model: &str, request: &ModelRequest) -> Value {
    json!({
        "model": model,
        "messages": request.messages.iter().map(chat_message_json).collect::<Vec<_>>(),
        "tools": request.tools.iter().map(chat_tool_json).collect::<Vec<_>>(),
        "parallel_tool_calls": false,
        "stream": true,
    })
}

fn responses_json(model: &str, request: &ModelRequest) -> Value {
    let mut input = Vec::new();
    for message in &request.messages {
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
    json!({
        "model": model,
        "input": input,
        "tools": request.tools.iter().map(responses_tool_json).collect::<Vec<_>>(),
        "parallel_tool_calls": false,
        "stream": true,
    })
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
