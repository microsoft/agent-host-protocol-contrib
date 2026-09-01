//! AHP schema fragments for the fixed OpenAI model-selection configuration.

use std::collections::HashMap;

use ahp_types::common::AnyValue;
use ahp_types::state::{ConfigPropertySchema, ConfigSchema, ModelSelection, SessionModelInfo};
use serde_json::Value;

const THINKING_LEVELS: [(&str, &str, &str); 7] = [
    ("none", "No thinking", "Respond without extended reasoning."),
    (
        "minimal",
        "Minimal thinking",
        "Use a small amount of reasoning.",
    ),
    (
        "low",
        "Low thinking",
        "Use concise reasoning before responding.",
    ),
    (
        "medium",
        "Medium thinking",
        "Balance reasoning depth and response time.",
    ),
    (
        "high",
        "High thinking",
        "Use more thorough reasoning for complex work.",
    ),
    (
        "xhigh",
        "Extra high thinking",
        "Use extensive reasoning for difficult work.",
    ),
    (
        "max",
        "Maximum thinking",
        "Use the greatest available reasoning effort.",
    ),
];

pub(super) fn standard_thinking_levels() -> Vec<String> {
    THINKING_LEVELS
        .iter()
        .map(|(level, _, _)| (*level).into())
        .collect()
}

pub(super) fn valid_thinking_level(value: &str) -> bool {
    THINKING_LEVELS
        .iter()
        .any(|(allowed, _, _)| *allowed == value)
}

pub(super) fn model_info(id: String, name: String, thinking_levels: &[String]) -> SessionModelInfo {
    SessionModelInfo {
        name,
        id,
        provider: "openai".into(),
        max_context_window: None,
        max_output_tokens: None,
        max_prompt_tokens: None,
        supports_vision: None,
        policy_state: None,
        config_schema: (!thinking_levels.is_empty()).then(|| ConfigSchema {
            r#type: "object".into(),
            properties: HashMap::from([(
                "thinkingLevel".into(),
                thinking_property(thinking_levels),
            )]),
            required: None,
        }),
        meta: None,
    }
}

pub(super) fn model_selection_properties(
    models: &[SessionModelInfo],
    selected_model: &str,
) -> HashMap<String, ConfigPropertySchema> {
    HashMap::from([
        (
            "id".into(),
            ConfigPropertySchema {
                r#type: "string".into(),
                title: "Model".into(),
                description: Some("Choose an OpenAI-compatible model.".into()),
                default: None,
                r#enum: Some(
                    models
                        .iter()
                        .map(|model| Value::String(model.id.clone()))
                        .collect(),
                ),
                enum_labels: Some(models.iter().map(|model| model.name.clone()).collect()),
                enum_descriptions: Some(
                    models
                        .iter()
                        .map(|model| format!("OpenAI-compatible model {}", model.id))
                        .collect(),
                ),
                read_only: None,
                items: None,
                properties: None,
                required: None,
                additional_properties: None,
            },
        ),
        (
            "config".into(),
            ConfigPropertySchema {
                r#type: "object".into(),
                title: "Model configuration".into(),
                description: Some("Options supported by the selected model.".into()),
                default: None,
                r#enum: None,
                enum_labels: None,
                enum_descriptions: None,
                read_only: None,
                items: None,
                properties: models
                    .iter()
                    .find(|model| model.id == selected_model)
                    .and_then(thinking_property_for)
                    .map(|thinking| HashMap::from([("thinkingLevel".into(), Box::new(thinking))])),
                required: None,
                additional_properties: None,
            },
        ),
    ])
}

pub(super) fn thinking_levels_for(model: &SessionModelInfo) -> Option<Vec<String>> {
    thinking_property_for(model).and_then(|property| {
        property.r#enum.and_then(|values| {
            values
                .into_iter()
                .map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
    })
}

fn thinking_property_for(model: &SessionModelInfo) -> Option<ConfigPropertySchema> {
    model
        .config_schema
        .as_ref()
        .and_then(|schema| schema.properties.get("thinkingLevel"))
        .cloned()
}

fn thinking_property(thinking_levels: &[String]) -> ConfigPropertySchema {
    ConfigPropertySchema {
        r#type: "string".into(),
        title: "Thinking level".into(),
        description: Some(
            "Controls reasoning effort. Thinking-level support is model-dependent; omit it to use the model default."
                .into(),
        ),
        default: None,
        r#enum: Some(
            thinking_levels
                .iter()
                .map(|value| AnyValue::String(value.clone()))
                .collect(),
        ),
        enum_labels: Some(
            thinking_levels
                .iter()
                .map(|value| thinking_metadata(value).1.into())
                .collect(),
        ),
        enum_descriptions: Some(
            thinking_levels
                .iter()
                .map(|value| thinking_metadata(value).2.into())
                .collect(),
        ),
        read_only: None,
        items: None,
        properties: None,
        required: None,
        additional_properties: None,
    }
}

fn thinking_metadata(value: &str) -> (&'static str, &'static str, &'static str) {
    THINKING_LEVELS
        .iter()
        .find(|(level, _, _)| *level == value)
        .copied()
        .unwrap_or(("unknown", "Unknown", "Unsupported thinking level."))
}

pub(super) fn selection_value(selection: &ModelSelection) -> AnyValue {
    serde_json::to_value(selection).expect("ModelSelection is serializable")
}
