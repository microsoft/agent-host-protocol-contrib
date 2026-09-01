use std::collections::HashSet;
use std::time::Duration;

use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::{ConformanceError, Probe};
use super::helpers::{
    model_selection, require_error, require_result, DEFAULT_MODEL, FAST_MODEL, REASONING_MODEL,
};

const EFFORTS: &[&str] = &["none", "minimal", "low", "medium", "high", "xhigh", "max"];

pub(super) async fn model_catalog(
    target: &str,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let initialized = probe
        .initialize(
            &format!("model-catalog-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let root = require_result(&initialized, "initialize")?
        .pointer("/snapshots/0/state")
        .ok_or_else(|| {
            ConformanceError::Options("initialize omitted root snapshot state".into())
        })?;
    let agent = root
        .get("agents")
        .and_then(Value::as_array)
        .and_then(|agents| {
            agents
                .iter()
                .find(|agent| agent.get("provider").and_then(Value::as_str) == Some("openai"))
        })
        .ok_or_else(|| {
            ConformanceError::Options("root did not advertise an OpenAI agent".into())
        })?;
    let models = agent
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("OpenAI agent omitted models".into()))?;
    let ids = models
        .iter()
        .map(|model| {
            model
                .get("id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| {
                    ConformanceError::Options("model catalog omitted a string id".into())
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let unique = ids.iter().collect::<HashSet<_>>();
    if ids != [FAST_MODEL, DEFAULT_MODEL, REASONING_MODEL] || unique.len() != ids.len() {
        return Err(ConformanceError::Options(format!(
            "fixture model catalog was {ids:?}, expected deterministic unique entries"
        )));
    }
    for (id, expected_levels) in [
        (FAST_MODEL, &["none", "minimal", "low"][..]),
        (DEFAULT_MODEL, &["minimal", "low", "medium", "high"][..]),
        (
            REASONING_MODEL,
            &["low", "medium", "high", "xhigh", "max"][..],
        ),
    ] {
        let model = models
            .iter()
            .find(|model| model.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| ConformanceError::Options(format!("catalog omitted {id:?}")))?;
        assert_thinking_schema(model, id, expected_levels)?;
    }
    probe.close().await?;
    Ok("discovers the deterministic OpenAI catalog with model-specific reasoning schemas".into())
}

pub(super) async fn session_model_config(
    target: &str,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    probe
        .initialize(
            &format!("session-model-config-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let default = probe
        .request(
            "resolveSessionConfig",
            json!({ "channel": "ahp-root://", "provider": "openai" }),
        )
        .await?;
    assert_resolved(&default, &model_selection(DEFAULT_MODEL, None))?;
    let selected = model_selection(REASONING_MODEL, Some("high"));
    let resolved = probe
        .request(
            "resolveSessionConfig",
            json!({
                "channel": "ahp-root://",
                "provider": "openai",
                "config": { "model": selected },
            }),
        )
        .await?;
    assert_resolved(&resolved, &selected)?;
    for (label, params) in [
        (
            "unknown model",
            json!({ "channel": "ahp-root://", "config": { "model": { "id": "missing" } } }),
        ),
        (
            "unsupported thinking level",
            json!({ "channel": "ahp-root://", "config": { "model": {
                "id": FAST_MODEL, "config": { "thinkingLevel": "high" }
            } } }),
        ),
        (
            "unknown provider",
            json!({ "channel": "ahp-root://", "provider": "missing" }),
        ),
        (
            "non-root channel",
            json!({ "channel": "ahp-session:/wrong" }),
        ),
        (
            "invalid workspace",
            json!({
                "channel": "ahp-root://",
                "workingDirectory": "file:///C:/missing-ahp-conformance-workspace",
            }),
        ),
    ] {
        let reply = probe.request("resolveSessionConfig", params).await?;
        require_error(&reply, &format!("resolveSessionConfig {label}"))?;
    }
    let completions = probe
        .request(
            "sessionConfigCompletions",
            json!({
                "channel": "ahp-root://",
                "provider": "openai",
                "property": "model",
                "query": "reasoning",
            }),
        )
        .await?;
    let items = require_result(&completions, "sessionConfigCompletions")?
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("model completions omitted items".into()))?;
    if items.len() != 1
        || items[0].get("value").and_then(Value::as_str) != Some(REASONING_MODEL)
        || items[0]
            .get("label")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || items[0]
            .get("description")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
    {
        return Err(ConformanceError::Options(
            "model completions were not filtered typed fixture-reasoning items".into(),
        ));
    }
    let unknown_property = probe
        .request(
            "sessionConfigCompletions",
            json!({ "channel": "ahp-root://", "property": "thinkingLevel" }),
        )
        .await?;
    require_error(&unknown_property, "unknown completion property")?;
    probe.close().await?;
    Ok("resolves canonical model config, validates contexts, and filters typed completions".into())
}

fn assert_thinking_schema(
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

fn assert_resolved(reply: &Value, expected: &Value) -> Result<(), ConformanceError> {
    let result = require_result(reply, "resolveSessionConfig")?;
    if result.pointer("/values/model") != Some(expected) {
        return Err(ConformanceError::Options(format!(
            "resolved values.model was {:?}, expected {expected}",
            result.pointer("/values/model")
        )));
    }
    let model = result
        .pointer("/schema/properties/model")
        .and_then(Value::as_object)
        .ok_or_else(|| ConformanceError::Options("resolved schema omitted model".into()))?;
    if model.get("type").and_then(Value::as_str) != Some("object")
        || model.get("sessionMutable").and_then(Value::as_bool) != Some(true)
        || model
            .pointer("/properties/id/enum")
            .and_then(Value::as_array)
            .is_none()
    {
        return Err(ConformanceError::Options(
            "resolveSessionConfig returned an incomplete model schema".into(),
        ));
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
