use std::collections::HashMap;
use std::sync::Arc;

use ahp_server_core::{
    CreateSession, HostStore, MarkSessionReady, ModelCatalog, SnapshotRequest, UpdateSessionConfig,
};
use ahp_types::state::{ModelSelection, SnapshotState};
use serde_json::{json, Value};

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
fn memory_session_schema_is_stable_across_model_changes() -> Result<(), Box<dyn std::error::Error>>
{
    let store = Arc::new(ahp_server_core::MemoryHostStore::new());
    let catalog = catalog()?;
    store.create_session(CreateSession {
        session: "ahp-session:/stable-memory".into(),
        default_chat: "ahp-chat:/stable-memory".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(selection("reasoning", Some("medium"))),
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: "ahp-session:/stable-memory".into(),
    })?;
    let before = session_schema(&store, &catalog)?;
    store.update_session_config(UpdateSessionConfig {
        session: "ahp-session:/stable-memory".into(),
        model_selection: selection("fast", None),
        origin: None,
    })?;
    let after = session_schema(&store, &catalog)?;
    assert_eq!(before, after);
    assert_eq!(
        before.pointer("/properties/model/properties/config/properties/thinkingLevel/enum"),
        Some(&json!(["medium"]))
    );
    Ok(())
}

fn session_schema(
    store: &ahp_server_core::MemoryHostStore,
    catalog: &ModelCatalog,
) -> Result<Value, Box<dyn std::error::Error>> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: "ahp-session:/stable-memory".into(),
        model_catalog: catalog.clone(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Session(state) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    serde_json::to_value(state.config.ok_or("missing session config")?.schema).map_err(Into::into)
}
