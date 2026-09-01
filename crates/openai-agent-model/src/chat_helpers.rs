//! Value validation helpers shared by the Chat Completions parser.

use serde_json::Value;

use crate::sse::ParsedEvent;
use crate::ModelError;

pub(crate) fn empty() -> ParsedEvent {
    ParsedEvent {
        events: Vec::new(),
        terminal: false,
    }
}

pub(crate) fn json_index(value: &Value) -> Result<usize, ModelError> {
    value
        .as_u64()
        .and_then(|index| index.try_into().ok())
        .ok_or_else(|| invalid("stream item index must be a non-negative integer"))
}

pub(crate) fn optional_non_empty_string(
    value: Option<&Value>,
    label: &str,
) -> Result<Option<String>, ModelError> {
    match value {
        None => Ok(None),
        Some(Value::String(value)) if !value.trim().is_empty() => Ok(Some(value.clone())),
        Some(Value::String(_)) => Err(invalid(&format!("{label} must not be empty"))),
        Some(_) => Err(invalid(&format!("{label} must be a string"))),
    }
}

pub(crate) fn update_identity(
    current: &mut Option<String>,
    next: Option<String>,
    label: &str,
) -> Result<(), ModelError> {
    if let Some(next) = next {
        if current.as_ref().is_some_and(|current| current != &next) {
            return Err(invalid(&format!("{label} changed within one stream")));
        }
        *current = Some(next);
    }
    Ok(())
}

pub(crate) fn invalid(message: &str) -> ModelError {
    ModelError::InvalidResponse(message.to_owned())
}
