use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::tools::workspace::file_uri;
use super::super::{ConformanceError, Probe};
use super::helpers::{
    model_selection, require_error, require_result, DEFAULT_MODEL, FAST_MODEL, REASONING_MODEL,
};
use super::schema::assert_thinking_schema;

pub(in super::super) async fn model_catalog(
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

pub(in super::super) async fn session_model_config(
    target: &str,
    timeout: Duration,
    workspace_root: &Path,
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
    let missing_workspace =
        file_uri(&workspace_root.join(format!("missing-workspace-{}", Uuid::new_v4())))?;
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
                "workingDirectory": missing_workspace,
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
            .get("properties")
            .and_then(|properties| properties.get("id"))
            .and_then(|id| id.get("enum"))
            .and_then(Value::as_array)
            .is_none()
        || !schema_supports_expected_thinking(model, expected)
    {
        return Err(ConformanceError::Options(
            "resolveSessionConfig returned an incomplete model schema".into(),
        ));
    }
    Ok(())
}

fn schema_supports_expected_thinking(
    model: &serde_json::Map<String, Value>,
    expected: &Value,
) -> bool {
    let thinking = model
        .get("properties")
        .and_then(|properties| properties.get("config"))
        .and_then(|config| config.get("properties"))
        .and_then(|properties| properties.get("thinkingLevel"))
        .and_then(|thinking| thinking.get("enum"))
        .and_then(Value::as_array);
    match expected
        .pointer("/config/thinkingLevel")
        .and_then(Value::as_str)
    {
        Some(expected) => thinking
            .is_some_and(|values| values.iter().any(|value| value.as_str() == Some(expected))),
        None => thinking.is_some(),
    }
}
