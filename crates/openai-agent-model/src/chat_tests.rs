use serde_json::json;

use super::ChatStreamParser;
use crate::sse::SseEvent;
use crate::{ModelError, ModelStreamEvent};

#[test]
fn emits_incremental_text_and_tool_arguments() -> Result<(), String> {
    let mut parser = ChatStreamParser::default();
    let text = parser
        .parse(event(
            json!({"choices":[{"index":0,"delta":{"content":"hel"},"finish_reason":null}]}),
        ))
        .map_err(|error| error.to_string())?;
    assert_eq!(text.events, [ModelStreamEvent::TextDelta("hel".to_owned())]);
    let started = parser
        .parse(event(json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call-1","type":"function","function":{"name":"weather","arguments":"{\"city\":"}}]}}]})))
        .map_err(|error| error.to_string())?;
    assert_eq!(
        started.events,
        [
            ModelStreamEvent::ToolCallStarted {
                index: 1,
                id: "call-1".to_owned(),
                name: "weather".to_owned(),
            },
            ModelStreamEvent::ToolCallArgumentsDelta {
                index: 1,
                delta: r#"{"city":"#.to_owned(),
            },
        ]
    );
    parser
        .parse(event(json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"Seattle\"}"}}]}}]})))
        .map_err(|error| error.to_string())?;
    parser
        .parse(event(
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}),
        ))
        .map_err(|error| error.to_string())?;
    assert!(
        parser
            .parse(SseEvent {
                event: None,
                data: "[DONE]".to_owned(),
            })
            .map_err(|error| error.to_string())?
            .terminal
    );
    Ok(())
}

#[test]
fn rejects_terminal_before_finish_and_mutating_ids() {
    let mut parser = ChatStreamParser::default();
    assert!(matches!(
        parser.parse(SseEvent {
            event: None,
            data: "[DONE]".to_owned(),
        }),
        Err(ModelError::InvalidResponse(_))
    ));
    let mut parser = ChatStreamParser::default();
    let start = event(
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"one","type":"function","function":{"name":"x","arguments":""}}]}}]}),
    );
    assert!(parser.parse(start).is_ok());
    let changed = event(
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"two","function":{"arguments":"{}"}}]}}]}),
    );
    assert!(matches!(
        parser.parse(changed),
        Err(ModelError::InvalidResponse(message)) if message.contains("changed")
    ));
    assert!(matches!(
        parser.parse(SseEvent {
            event: None,
            data: "{not-json".to_owned(),
        }),
        Err(ModelError::InvalidResponse(message)) if message.contains("valid JSON")
    ));
}

fn event(value: serde_json::Value) -> SseEvent {
    SseEvent {
        event: None,
        data: value.to_string(),
    }
}
