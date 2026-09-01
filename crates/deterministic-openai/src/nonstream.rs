//! Non-streaming fixture response DTO serialization.

use axum::response::Response;
use serde::Serialize;
use serde_json::json;

use super::response::delayed_json_response;
use super::v2::V2Action;
use super::{V2_CHAT_COMPLETION_ID, V2_RESPONSES_ID};

#[derive(Serialize)]
struct ChatChoice {
    index: usize,
    message: ChatMessage,
    finish_reason: &'static str,
}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<ChatToolCall>,
}

#[derive(Serialize)]
struct ChatToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: &'static str,
    function: ChatFunctionCall,
}

#[derive(Serialize)]
struct ChatFunctionCall {
    name: String,
    arguments: String,
}

pub(super) fn chat_text(completion_id: &str, content: String, delay_ms: u64) -> Response {
    delayed_json_response(
        json!({
            "id": completion_id,
            "object": "chat.completion",
            "choices": [ChatChoice {
                index: 0,
                message: ChatMessage {
                    role: "assistant",
                    content: Some(content),
                    tool_calls: Vec::new(),
                },
                finish_reason: "stop",
            }],
        }),
        delay_ms,
    )
}

pub(super) fn v2_chat(action: V2Action, delay_ms: u64) -> Response {
    let (content, tool_calls, finish_reason) = match action {
        V2Action::ToolCall {
            id,
            name,
            arguments,
        } => (
            None,
            vec![ChatToolCall {
                id,
                kind: "function",
                function: ChatFunctionCall { name, arguments },
            }],
            "tool_calls",
        ),
        V2Action::FinalText(content) => (Some(content), Vec::new(), "stop"),
    };
    delayed_json_response(
        json!({
            "id": V2_CHAT_COMPLETION_ID,
            "object": "chat.completion",
            "choices": [ChatChoice {
                index: 0,
                message: ChatMessage { role: "assistant", content, tool_calls },
                finish_reason,
            }],
        }),
        delay_ms,
    )
}

pub(super) fn responses_text(response_id: &str, content: String, delay_ms: u64) -> Response {
    delayed_json_response(
        json!({
            "id": response_id,
            "object": "response",
            "created_at": 0,
            "status": "completed",
            "output": [{
                "id": format!("{response_id}-message-0"),
                "type": "message",
                "role": "assistant",
                "status": "completed",
                "content": [{
                    "type": "output_text",
                    "text": content,
                    "annotations": [],
                    "logprobs": [],
                }],
            }],
        }),
        delay_ms,
    )
}

pub(super) fn v2_responses(action: V2Action, delay_ms: u64) -> Response {
    match action {
        V2Action::ToolCall {
            id,
            name,
            arguments,
        } => delayed_json_response(
            json!({
                "id": V2_RESPONSES_ID,
                "object": "response",
                "created_at": 0,
                "status": "completed",
                "output": [{
                    "id": format!("{V2_RESPONSES_ID}-function-0"),
                    "type": "function_call",
                    "call_id": id,
                    "name": name,
                    "arguments": arguments,
                    "status": "completed",
                }],
            }),
            delay_ms,
        ),
        V2Action::FinalText(content) => responses_text(V2_RESPONSES_ID, content, delay_ms),
    }
}
