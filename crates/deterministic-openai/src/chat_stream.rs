//! Chat Completions SSE serialization for fixture text and function calls.

use axum::response::Response;
use serde_json::{json, Value};

use super::response::{sse_response, FixtureSseEvent};

pub(super) fn text(completion_id: &str, chunks: Vec<String>, delay_ms: u64) -> Response {
    let mut events = chunks
        .into_iter()
        .map(|chunk| {
            FixtureSseEvent::json(
                None,
                json!({
                    "id": completion_id,
                    "object": "chat.completion.chunk",
                    "choices": [{"index": 0, "delta": {"content": chunk}, "finish_reason": Value::Null}],
                }),
                true,
            )
        })
        .collect::<Vec<_>>();
    events.push(FixtureSseEvent::json(
        None,
        json!({
            "id": completion_id,
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}],
        }),
        false,
    ));
    events.push(FixtureSseEvent::raw(None, "[DONE]", false));
    sse_response(events, delay_ms)
}

pub(super) fn tool_call(
    completion_id: &str,
    index: usize,
    id: String,
    name: String,
    argument_chunks: Vec<String>,
    delay_ms: u64,
) -> Response {
    let mut events = vec![FixtureSseEvent::json(
        None,
        json!({
            "id": completion_id,
            "object": "chat.completion.chunk",
            "choices": [{
                "index": 0,
                "delta": {"tool_calls": [{
                    "index": index,
                    "id": id,
                    "type": "function",
                    "function": {"name": name, "arguments": ""}
                }]},
                "finish_reason": Value::Null,
            }],
        }),
        false,
    )];
    events.extend(argument_chunks.into_iter().map(|chunk| {
        FixtureSseEvent::json(
            None,
            json!({
                "id": completion_id,
                "object": "chat.completion.chunk",
                "choices": [{
                    "index": 0,
                    "delta": {"tool_calls": [{
                        "index": index,
                        "function": {"arguments": chunk}
                    }]},
                    "finish_reason": Value::Null,
                }],
            }),
            true,
        )
    }));
    events.push(FixtureSseEvent::json(
        None,
        json!({
            "id": completion_id,
            "object": "chat.completion.chunk",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}],
        }),
        false,
    ));
    events.push(FixtureSseEvent::raw(None, "[DONE]", false));
    sse_response(events, delay_ms)
}
