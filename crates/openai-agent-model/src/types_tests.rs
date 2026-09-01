use std::str::FromStr;

use crate::{ModelSelection, ReasoningEffort, ReasoningEffortParseError};

#[test]
fn reasoning_effort_uses_openai_wire_values_and_rejects_unknown_values() {
    for (value, expected) in [
        ("none", ReasoningEffort::None),
        ("minimal", ReasoningEffort::Minimal),
        ("low", ReasoningEffort::Low),
        ("medium", ReasoningEffort::Medium),
        ("high", ReasoningEffort::High),
        ("xhigh", ReasoningEffort::XHigh),
        ("max", ReasoningEffort::Max),
    ] {
        assert_eq!(ReasoningEffort::from_str(value), Ok(expected));
        assert_eq!(expected.to_string(), value);
    }
    assert!(matches!(
        ReasoningEffort::from_str("unsupported"),
        Err(ReasoningEffortParseError { .. })
    ));
}

#[test]
fn reasoning_effort_deserializes_only_known_wire_values() {
    let value: Result<ReasoningEffort, _> = serde_json::from_str("\"medium\"");
    assert!(matches!(value, Ok(ReasoningEffort::Medium)));
    let invalid: Result<ReasoningEffort, _> = serde_json::from_str("\"unsupported\"");
    assert!(invalid.is_err());

    let selection: Result<ModelSelection, _> =
        serde_json::from_str(r#"{"id":"fixture-model","reasoning_effort":"unsupported"}"#);
    assert!(selection.is_err());
}
