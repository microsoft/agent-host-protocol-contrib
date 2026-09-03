use std::sync::{Arc, Mutex};

use ahp_server_core::{
    ActionReplay, CreateSession, HostStore, MarkSessionReady, ModelCatalog, SnapshotRequest,
};
use ahp_types::state::{ModelSelection, SnapshotState};
use serde_json::json;

use super::{action, host_with_catalog, notify, request, RecordingModel};

#[tokio::test]
async fn rejects_a_stale_saved_model_before_starting_or_calling_upstream(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(ahp_server_core::MemoryHostStore::new());
    store.create_session(CreateSession {
        session: "ahp-session:/stale".into(),
        default_chat: "ahp-chat:/stale".into(),
        provider: "openai".into(),
        workspace: "file:///workspace".into(),
        model_selection: Some(ModelSelection {
            id: "removed-model".into(),
            config: None,
        }),
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: "ahp-session:/stale".into(),
    })?;
    let model = Arc::new(RecordingModel {
        requests: Mutex::new(Vec::new()),
        default_model: Some("alpha".into()),
    });
    let catalog = ModelCatalog::single("available-model");
    let host = host_with_catalog(catalog.clone(), store.clone(), model.clone())?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["0.8.0"],
            "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "subscribe",
        json!({ "channel": "ahp-chat:/stale" }),
    )
    .await?;
    notify(
        &host,
        &peer,
        "ahp-chat:/stale",
        3,
        json!({
            "type": "chat/turnStarted", "turnId": "stale",
            "startedAt": "2026-09-01T00:00:00.000Z",
            "message": { "text": "must not run", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    let rejected = action(&mut peer, "chat/turnStarted").await?;
    assert!(rejected.pointer("/params/rejectionReason").is_some());
    assert!(model.requests().is_empty());
    let ActionReplay::Replay(replay) = store.replay_since(0)? else {
        return Err("expected replayable actions".into());
    };
    assert!(replay.iter().all(|envelope| {
        !matches!(
            envelope.action,
            ahp_types::actions::StateAction::ChatTurnStarted(_)
        ) || envelope.rejection_reason.is_some()
    }));
    notify(
        &host,
        &peer,
        "ahp-chat:/stale",
        4,
        json!({
            "type": "chat/turnStarted", "turnId": "override",
            "startedAt": "2026-09-01T00:01:00.000Z",
            "message": {
                "text": "override remains available", "origin": { "kind": "user" },
                "model": { "id": "available-model" }
            }
        }),
    )
    .await?;
    let accepted = action(&mut peer, "chat/turnStarted").await?;
    assert!(accepted.pointer("/params/rejectionReason").is_none());
    assert_eq!(
        accepted.pointer("/params/action/message/model/id"),
        Some(&json!("available-model"))
    );
    let snapshot = store.snapshot(SnapshotRequest {
        channel: "ahp-session:/stale".into(),
        model_catalog: catalog,
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
