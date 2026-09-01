//! Deterministic JSON serialization for v2 function-call arguments.

use serde_json::Value;

use super::contract::{code, ContractError};

pub(super) fn canonical_json(value: &Value) -> Result<String, ContractError> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            serde_json::to_string(value).map_err(|_| serialization_error())
        }
        Value::Array(values) => Ok(format!(
            "[{}]",
            values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        )),
        Value::Object(values) => {
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_unstable_by_key(|(key, _)| *key);
            Ok(format!(
                "{{{}}}",
                entries
                    .into_iter()
                    .map(|(key, value)| {
                        Ok(format!(
                            "{}:{}",
                            serde_json::to_string(key).map_err(|_| serialization_error())?,
                            canonical_json(value)?
                        ))
                    })
                    .collect::<Result<Vec<_>, ContractError>>()?
                    .join(",")
            ))
        }
    }
}

fn serialization_error() -> ContractError {
    ContractError::new(
        code::INVALID_DIRECTIVE,
        "tool-call JSON could not be serialized deterministically",
    )
}
