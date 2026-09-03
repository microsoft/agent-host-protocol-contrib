use std::collections::HashMap;

use ahp_server_core::{
    CreateSession, HostStore, MarkSessionReady, ModelCatalog, SnapshotRequest, UpdateSessionConfig,
};
use ahp_types::state::{ModelSelection, SnapshotState};
use serde_json::{json, Value};

use super::Fixture;

fn catalog() -> Result<ModelCatalog, Box<dyn std::error::Error>> {
    Ok(ModelCatalog {
        models: vec![
            ModelCatalog::model_info_with_thinking_levels("reasoning", "Reasoning", ["medium"])?,
            ModelCatalog::model_info_with_thinking_levels(
                "fast",
                "Fast",
                std::iter::empty::<&str>(),
            )?,
        ],
        default_selection: Some(selection("reasoning", Some("medium"))),
    })
}

fn selection(id: &str, thinking_level: Option<&str>) -> ModelSelection {
    ModelSelection {
        id: id.into(),
        config: thinking_level
            .map(|level| HashMap::from([("thinkingLevel".into(), Value::String(level.into()))])),
    }
}

#[test]
fn redb_session_schema_is_stable_across_reasoning_to_fast_changes(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    let catalog = catalog()?;
    fixture.store.create_session(CreateSession {
        session: "ahp-session:/stable-redb".into(),
        default_chat: "ahp-chat:/stable-redb".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("reasoning", Some("medium"))),
    })?;
    fixture.store.mark_session_ready(MarkSessionReady {
        session: "ahp-session:/stable-redb".into(),
    })?;
    let before = session_schema(&fixture, &catalog)?;
    fixture.store.update_session_config(UpdateSessionConfig {
        session: "ahp-session:/stable-redb".into(),
        model_selection: selection("fast", None),
        origin: None,
    })?;
    fixture.reopen()?;
    let after = session_schema(&fixture, &catalog)?;
    assert_eq!(before, after);
    assert_eq!(
        before.pointer("/properties/model/properties/config/properties/thinkingLevel/enum"),
        Some(&json!(["medium"]))
    );
    Ok(())
}

#[test]
fn redb_catalog_change_keeps_stale_selection_visible_after_restart(
) -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Fixture::new()?;
    fixture.store.create_session(CreateSession {
        session: "ahp-session:/stale-redb".into(),
        default_chat: "ahp-chat:/stale-redb".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(ModelSelection {
            id: "removed-model".into(),
            config: None,
        }),
    })?;
    fixture.store.mark_session_ready(MarkSessionReady {
        session: "ahp-session:/stale-redb".into(),
    })?;
    fixture.reopen()?;
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: "ahp-session:/stale-redb".into(),
        model_catalog: ModelCatalog::single("available-model"),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    assert_eq!(
        state
            .config
            .and_then(|config| config.values.get("model").cloned()),
        Some(json!({ "id": "removed-model" }))
    );
    Ok(())
}

fn session_schema(
    fixture: &Fixture,
    catalog: &ModelCatalog,
) -> Result<Value, Box<dyn std::error::Error>> {
    let snapshot = fixture.store.snapshot(SnapshotRequest {
        channel: "ahp-session:/stable-redb".into(),
        model_catalog: catalog.clone(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    serde_json::to_value(state.config.ok_or("missing session config")?.schema).map_err(Into::into)
}
