//! DTO validation helpers for streamed Responses API events.

use serde_json::{Map, Value};

use crate::sse::{ParsedEvent, SseEvent};
use crate::ModelError;

pub(crate) fn event_type<'a>(
    event: &'a SseEvent,
    object: &'a Map<String, Value>,
) -> Result<&'a str, ModelError> {
    let data_type = object.get("type").and_then(Value::as_str);
    match (event.event.as_deref(), data_type) {
        (Some(event_type), Some(data_type)) if event_type != data_type => {
            Err(invalid("SSE event name and Responses data type disagree"))
        }
        (Some(event_type), _) => Ok(event_type),
        (None, Some(data_type)) => Ok(data_type),
        (None, None) => Err(invalid(
            "Responses SSE event requires an event name or data type",
        )),
    }
}

pub(crate) fn output_index(object: &Map<String, Value>) -> Result<usize, ModelError> {
    object
        .get("output_index")
        .and_then(Value::as_u64)
        .and_then(|index| index.try_into().ok())
        .ok_or_else(|| invalid("Responses event output_index must be a non-negative integer"))
}

pub(crate) fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a Map<String, Value>, ModelError> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| invalid(&format!("{context} requires object field {field:?}")))
}

pub(crate) fn required_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<&'a str, ModelError> {
    optional_string(object, field, context)?
        .ok_or_else(|| invalid(&format!("{context} requires string field {field:?}")))
}

pub(crate) fn required_non_empty_string(
    object: &Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<String, ModelError> {
    let value = required_string(object, field, context)?;
    (!value.trim().is_empty())
        .then(|| value.to_owned())
        .ok_or_else(|| invalid(&format!("{context} field {field:?} must not be empty")))
}

pub(crate) fn optional_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    context: &str,
) -> Result<Option<&'a str>, ModelError> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(invalid(&format!(
            "{context} field {field:?} must be a string"
        ))),
    }
}

pub(crate) fn failure_message(object: &Map<String, Value>) -> String {
    let message = object
        .get("error")
        .and_then(Value::as_object)
        .and_then(|error| error.get("message"))
        .or_else(|| object.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("Responses stream reported a failure");
    message.chars().take(4_096).collect()
}

pub(crate) fn empty() -> ParsedEvent {
    ParsedEvent {
        events: Vec::new(),
        terminal: false,
    }
}

pub(crate) fn invalid(message: &str) -> ModelError {
    ModelError::InvalidResponse(message.to_owned())
}
