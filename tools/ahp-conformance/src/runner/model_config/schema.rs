use serde_json::Value;

use super::super::ConformanceError;

const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub(super) fn assert_thinking_schema(
    model: &Value,
    id: &str,
    expected_levels: &[&str],
) -> Result<(), ConformanceError> {
    let property = model
        .pointer("/configSchema/properties/thinkingLevel")
        .and_then(Value::as_object)
        .ok_or_else(|| ConformanceError::Options(format!("{id:?} lacks thinkingLevel schema")))?;
    let values = string_array(property.get("enum"), id, "enum")?;
    if values != expected_levels {
        return Err(ConformanceError::Options(format!(
            "{id:?} thinking-level enum was {values:?}, expected {expected_levels:?}"
        )));
    }
    if values.iter().any(|value| !EFFORTS.contains(value)) {
        return Err(ConformanceError::Options(format!(
            "{id:?} advertised an unsupported reasoning effort"
        )));
    }
    for field in ["enumLabels", "enumDescriptions"] {
        let metadata = string_array(property.get(field), id, field)?;
        if metadata.len() != values.len() || metadata.iter().any(|value| value.is_empty()) {
            return Err(ConformanceError::Options(format!(
                "{id:?} {field} is not aligned with its enum"
            )));
        }
    }
    if property.get("type").and_then(Value::as_str) != Some("string")
        || property
            .get("title")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || property
            .get("description")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(ConformanceError::Options(format!(
            "{id:?} thinkingLevel schema is not production-shaped"
        )));
    }
    Ok(())
}

fn string_array<'a>(
    value: Option<&'a Value>,
    id: &str,
    field: &str,
) -> Result<Vec<&'a str>, ConformanceError> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options(format!("{id:?} lacks {field}")))
        .and_then(|values| {
            values
                .iter()
                .map(|value| {
                    value.as_str().ok_or_else(|| {
                        ConformanceError::Options(format!("{id:?} {field} has a non-string value"))
                    })
                })
                .collect()
        })
}
