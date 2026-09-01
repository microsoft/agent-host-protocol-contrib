//! Responses API SSE serialization for fixture text and function calls.

use axum::response::Response;
use serde_json::{json, Value};

use super::response::{responses_event, sse_response};

pub(super) fn text(response_id: &str, chunks: Vec<String>, delay_ms: u64) -> Response {
    let item_id = format!("{response_id}-message-0");
    let text = chunks.concat();
    let content = output_text(&text);
    let item = message_item(&item_id, vec![content.clone()], "completed");
    let mut events = vec![
        responses_event(
            "response.created",
            0,
            json!({"response": response_snapshot(response_id, vec![], "in_progress")}),
            false,
        ),
        responses_event(
            "response.in_progress",
            1,
            json!({"response": response_snapshot(response_id, vec![], "in_progress")}),
            false,
        ),
        responses_event(
            "response.output_item.added",
            2,
            json!({"output_index": 0, "item": message_item(&item_id, vec![], "in_progress")}),
            false,
        ),
        responses_event(
            "response.content_part.added",
            3,
            json!({"item_id": item_id, "output_index": 0, "content_index": 0, "part": output_text("")}),
            false,
        ),
    ];
    events.extend(chunks.into_iter().enumerate().map(|(index, chunk)| {
        responses_event(
            "response.output_text.delta",
            (index + 4) as u64,
            json!({
                "item_id": item_id,
                "output_index": 0,
                "content_index": 0,
                "delta": chunk,
                "logprobs": [],
            }),
            true,
        )
    }));
    let done_sequence = events.len() as u64;
    events.extend([
        responses_event(
            "response.output_text.done",
            done_sequence,
            json!({
                "item_id": item_id,
                "output_index": 0,
                "content_index": 0,
                "text": text,
                "logprobs": [],
            }),
            false,
        ),
        responses_event(
            "response.content_part.done",
            done_sequence + 1,
            json!({"item_id": item_id, "output_index": 0, "content_index": 0, "part": content}),
            false,
        ),
        responses_event(
            "response.output_item.done",
            done_sequence + 2,
            json!({"output_index": 0, "item": item.clone()}),
            false,
        ),
        responses_event(
            "response.completed",
            done_sequence + 3,
            json!({"response": response_snapshot(response_id, vec![item], "completed")}),
            false,
        ),
    ]);
    sse_response(events, delay_ms)
}

pub(super) fn tool_call(
    response_id: &str,
    index: usize,
    id: String,
    name: String,
    arguments: String,
    argument_chunks: Vec<String>,
    delay_ms: u64,
) -> Response {
    let item_id = format!("{response_id}-function-{index}");
    let completed_item = function_item(&item_id, &id, &name, &arguments, "completed");
    let mut events = vec![
        responses_event(
            "response.created",
            0,
            json!({"response": response_snapshot(response_id, vec![], "in_progress")}),
            false,
        ),
        responses_event(
            "response.in_progress",
            1,
            json!({"response": response_snapshot(response_id, vec![], "in_progress")}),
            false,
        ),
        responses_event(
            "response.output_item.added",
            2,
            json!({
                "output_index": index,
                "item": function_item(&item_id, &id, &name, "", "in_progress"),
            }),
            false,
        ),
    ];
    events.extend(
        argument_chunks
            .into_iter()
            .enumerate()
            .map(|(offset, delta)| {
                responses_event(
                    "response.function_call_arguments.delta",
                    (offset + 3) as u64,
                    json!({"item_id": item_id, "output_index": index, "delta": delta}),
                    true,
                )
            }),
    );
    let done_sequence = events.len() as u64;
    events.extend([
        responses_event(
            "response.function_call_arguments.done",
            done_sequence,
            json!({
                "item_id": item_id,
                "output_index": index,
                "name": name,
                "arguments": arguments,
            }),
            false,
        ),
        responses_event(
            "response.output_item.done",
            done_sequence + 1,
            json!({"output_index": index, "item": completed_item.clone()}),
            false,
        ),
        responses_event(
            "response.completed",
            done_sequence + 2,
            json!({"response": response_snapshot(response_id, vec![completed_item], "completed")}),
            false,
        ),
    ]);
    sse_response(events, delay_ms)
}

fn output_text(text: &str) -> Value {
    json!({"type": "output_text", "text": text, "annotations": [], "logprobs": []})
}

fn message_item(id: &str, content: Vec<Value>, status: &str) -> Value {
    json!({"id": id, "type": "message", "role": "assistant", "status": status, "content": content})
}

fn function_item(id: &str, call_id: &str, name: &str, arguments: &str, status: &str) -> Value {
    json!({
        "id": id,
        "type": "function_call",
        "call_id": call_id,
        "name": name,
        "arguments": arguments,
        "status": status,
    })
}

fn response_snapshot(response_id: &str, output: Vec<Value>, status: &str) -> Value {
    json!({
        "id": response_id,
        "object": "response",
        "created_at": 0,
        "status": status,
        "error": null,
        "incomplete_details": null,
        "instructions": null,
        "max_output_tokens": null,
        "model": "fixture-model",
        "output": output,
        "parallel_tool_calls": false,
        "previous_response_id": null,
        "reasoning": {"effort": null, "summary": null},
        "store": false,
        "temperature": 1.0,
        "text": {"format": {"type": "text"}},
        "tool_choice": "auto",
        "tools": [],
        "top_p": 1.0,
        "truncation": "disabled",
        "usage": null,
        "user": null,
        "metadata": {},
    })
}
