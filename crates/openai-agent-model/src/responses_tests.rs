use serde_json::json;

use super::ResponsesStreamParser;
use crate::sse::SseEvent;
use crate::{ModelError, ModelStreamEvent};

#[test]
fn emits_and_validates_function_call_lifecycle() -> Result<(), String> {
    let mut parser = ResponsesStreamParser::default();
    let added = parser
        .parse(event(
            "response.output_item.added",
            json!({"output_index":1,"item":{"type":"function_call","call_id":"call-1","name":"weather","arguments":""}}),
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        added.events,
        [ModelStreamEvent::ToolCallStarted {
            index: 1,
            id: "call-1".to_owned(),
            name: "weather".to_owned(),
        }]
    );
    let delta = parser
        .parse(event(
            "response.function_call_arguments.delta",
            json!({"output_index":1,"delta":"{}"}),
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        delta.events,
        [ModelStreamEvent::ToolCallArgumentsDelta {
            index: 1,
            delta: "{}".to_owned(),
        }]
    );
    parser
        .parse(event(
            "response.function_call_arguments.done",
            json!({"output_index":1,"arguments":"{}"}),
        ))
        .map_err(|error| error.to_string())?;
    parser
        .parse(event(
            "response.output_item.done",
            json!({"output_index":1,"item":{"type":"function_call","call_id":"call-1","name":"weather","arguments":"{}"}}),
        ))
        .map_err(|error| error.to_string())?;
    assert!(
        parser
            .parse(event("response.completed", json!({"response": {"id":"r"}})))
            .map_err(|error| error.to_string())?
            .terminal
    );
    Ok(())
}

#[test]
fn turns_error_events_into_typed_failures() {
    let mut parser = ResponsesStreamParser::default();
    assert!(matches!(
        parser.parse(event("error", json!({"message":"nope"}))),
        Err(ModelError::StreamFailed { event, message })
            if event == "error" && message == "nope"
    ));
}

#[test]
fn rejects_inconsistent_function_call_arguments() {
    let mut parser = ResponsesStreamParser::default();
    assert!(matches!(
        parser.parse(event(
            "response.function_call_arguments.delta",
            json!({"output_index": 0, "delta": "{}"}),
        )),
        Err(ModelError::InvalidResponse(message)) if message.contains("unknown output_index")
    ));
}

fn event(event: &'static str, mut value: serde_json::Value) -> SseEvent {
    if let Some(object) = value.as_object_mut() {
        object.insert("type".to_owned(), json!(event));
    }
    SseEvent {
        event: Some(event.to_owned()),
        data: value.to_string(),
    }
}
