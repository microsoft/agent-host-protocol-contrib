use std::sync::{Arc, Mutex};

use ahp_server_core::{
    ActionReplay, ChangeDraft, CommittedTransition, CreateChat, CreateSession, CreatedSession,
    DisposedSession, FinishTurn, HostStore, MarkSessionReady, ModelCatalog, SnapshotRequest,
    StartedTurn, StoreError, ToolCallTransition, TurnStart, UpdateSessionConfig,
};
use ahp_types::actions::{ActionOrigin, StateAction};
use ahp_types::common::Uri;
use ahp_types::state::{ChatSummary, ModelSelection, ResponsePart, SessionSummary, Snapshot};
use serde_json::json;
use workspace_agent_tools::ExactCommandApproval;

use super::{action, host_with_catalog, notify, request, wait_for_requests, RecordingModel};

struct NoSavedSelectionStore {
    inner: Arc<ahp_server_core::MemoryHostStore>,
}

impl NoSavedSelectionStore {
    fn unsupported<T>() -> Result<T, StoreError> {
        Err(StoreError::Backend(
            "unexpected test-store operation".into(),
        ))
    }
}

impl HostStore for NoSavedSelectionStore {
    fn server_sequence(&self) -> Result<i64, StoreError> {
        self.inner.server_sequence()
    }

    fn snapshot(&self, request: SnapshotRequest) -> Result<Snapshot, StoreError> {
        self.inner.snapshot(request)
    }

    fn replay_since(&self, sequence: i64) -> Result<ActionReplay, StoreError> {
        self.inner.replay_since(sequence)
    }

    fn list_sessions(&self) -> Result<Vec<SessionSummary>, StoreError> {
        self.inner.list_sessions()
    }

    fn session_summary(&self, session: &str) -> Result<SessionSummary, StoreError> {
        self.inner.session_summary(session)
    }

    fn model_selection_for_chat(
        &self,
        _: &str,
        _: Option<&ModelSelection>,
    ) -> Result<ModelSelection, StoreError> {
        Err(
            ahp_server_core::StoreConflict::MissingModelSelection("ahp-session:/legacy".into())
                .into(),
        )
    }

    fn create_session(
        &self,
        request: CreateSession,
    ) -> Result<CommittedTransition<CreatedSession>, StoreError> {
        self.inner.create_session(request)
    }

    fn update_session_config(
        &self,
        _: UpdateSessionConfig,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn change_draft(&self, _: ChangeDraft) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn mark_session_ready(
        &self,
        request: MarkSessionReady,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.inner.mark_session_ready(request)
    }

    fn create_chat(&self, _: CreateChat) -> Result<CommittedTransition<ChatSummary>, StoreError> {
        Self::unsupported()
    }

    fn dispose_session(&self, _: &str) -> Result<CommittedTransition<DisposedSession>, StoreError> {
        Self::unsupported()
    }

    fn start_turn(
        &self,
        request: TurnStart,
    ) -> Result<CommittedTransition<StartedTurn>, StoreError> {
        self.inner.start_turn(request)
    }

    fn append_response_part(
        &self,
        _: &str,
        _: &str,
        _: ResponsePart,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn append_text(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn start_tool(
        &self,
        _: &str,
        _: &str,
        _: ResponsePart,
        _: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn append_tool_delta(
        &self,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
        _: StateAction,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn transition_tool(
        &self,
        _: ToolCallTransition,
    ) -> Result<CommittedTransition<()>, StoreError> {
        Self::unsupported()
    }

    fn finish_turn(&self, request: FinishTurn) -> Result<CommittedTransition<i64>, StoreError> {
        self.inner.finish_turn(request)
    }

    fn reject_action(
        &self,
        channel: Uri,
        action: StateAction,
        origin: Option<ActionOrigin>,
        reason: String,
    ) -> Result<CommittedTransition<()>, StoreError> {
        self.inner.reject_action(channel, action, origin, reason)
    }

    fn is_command_allowed(&self, _: &ExactCommandApproval) -> Result<bool, StoreError> {
        Self::unsupported()
    }

    fn reset_command_approvals(&self) -> Result<(), StoreError> {
        Self::unsupported()
    }
}

#[tokio::test]
async fn explicit_message_model_bypasses_missing_legacy_selection(
) -> Result<(), Box<dyn std::error::Error>> {
    let memory = Arc::new(ahp_server_core::MemoryHostStore::new());
    let store = Arc::new(NoSavedSelectionStore {
        inner: memory.clone(),
    });
    let model = Arc::new(RecordingModel {
        requests: Mutex::new(Vec::new()),
        default_model: None,
    });
    let catalog = ModelCatalog::new([("alpha", "Alpha"), ("beta", "Beta")]);
    let host = host_with_catalog(catalog.clone(), store, model.clone())?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["0.9.0"],
            "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({
            "channel": "ahp-session:/legacy",
            "config": { "model": { "id": "alpha" } }
        }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/legacy" }),
    )
    .await?;
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("missing default chat")?
        .to_owned();
    request(&host, &mut peer, 4, "subscribe", json!({ "channel": chat })).await?;
    notify(
        &host,
        &peer,
        &chat,
        5,
        json!({
            "type": "chat/turnStarted", "turnId": "explicit",
            "startedAt": "2026-09-02T00:00:00.000Z",
            "message": {
                "text": "explicit model", "origin": { "kind": "user" },
                "model": { "id": "beta" }
            }
        }),
    )
    .await?;
    let accepted = action(&mut peer, "chat/turnStarted").await?;
    assert!(accepted.pointer("/params/rejectionReason").is_none());
    wait_for_requests(&model, 1).await?;
    let completed = action(&mut peer, "chat/turnComplete").await?;
    assert!(completed.pointer("/params/rejectionReason").is_none());

    let snapshot = memory.snapshot(SnapshotRequest {
        channel: chat.clone(),
        model_catalog: catalog.clone(),
        server_tools: Vec::new(),
    })?;
    let ahp_types::state::SnapshotState::Chat(snapshot) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    assert_eq!(
        snapshot.turns[0].message.model,
        Some(ModelSelection {
            id: "beta".into(),
            config: None,
        })
    );
    let snapshot = memory.snapshot(SnapshotRequest {
        channel: "ahp-session:/legacy".into(),
        model_catalog: catalog,
        server_tools: Vec::new(),
    })?;
    let ahp_types::state::SnapshotState::Session(snapshot) = snapshot.state else {
        return Err("expected session snapshot".into());
    };
    assert_eq!(
        snapshot
            .config
            .and_then(|config| config.values.get("model").cloned()),
        Some(json!({ "id": "alpha" }))
    );
    notify(
        &host,
        &peer,
        &chat,
        6,
        json!({
            "type": "chat/turnStarted", "turnId": "omitted",
            "startedAt": "2026-09-02T00:01:00.000Z",
            "message": { "text": "omitted model", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    let rejected = action(&mut peer, "chat/turnStarted").await?;
    assert!(rejected.pointer("/params/rejectionReason").is_some());
    assert_eq!(model.requests().len(), 1);
    Ok(())
}
