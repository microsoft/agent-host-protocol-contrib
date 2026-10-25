use std::sync::Arc;
use std::time::Duration;

use ahp_types::actions::ChatTurnStartedAction;
use ahp_types::state::{ResponsePart, SnapshotState, ToolCallConfirmationReason, ToolCallState};
use futures_util::stream;
use openai_agent_model::{
    AgentModel, ModelEventStream, ModelRequest, ModelRequestFuture, ModelToolCall,
};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::ConfiguredWorkspaces;

use super::{execute_client_tool, PendingClientTool};
use crate::host::CoreHost;
use crate::presentation::ToolPresentation;
use crate::{
    ApprovalKey, ChangeSession, CoreConfig, CreateChat, CreateSession, HostStore, MarkSessionReady,
    MemoryHostStore, ModelCatalog, SessionMutation, SnapshotRequest, ToolCallTransition, TurnStart,
    WorkspaceAgentProfile,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct NeverModel;

impl AgentModel for NeverModel {
    fn default_model(&self) -> Option<&str> {
        Some("test")
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        Box::pin(async {
            let events: ModelEventStream = Box::pin(stream::pending());
            Ok(events)
        })
    }
}

fn tool_state(store: &MemoryHostStore, chat: &str) -> Result<ToolCallState> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: chat.into(),
        model_catalog: ModelCatalog::single("test"),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat".into());
    };
    let turn = chat.active_turn.ok_or("expected active turn")?;
    turn.response_parts
        .into_iter()
        .find_map(|part| match part {
            ResponsePart::ToolCall(tool) => Some(tool.tool_call),
            _ => None,
        })
        .ok_or_else(|| "expected tool call".into())
}

#[tokio::test]
async fn stale_registration_fails_only_its_call_instead_of_other_chats() -> Result {
    let store = Arc::new(MemoryHostStore::new());
    let host = CoreHost::new(
        CoreConfig::default(),
        store.clone(),
        Arc::new(NeverModel),
        Arc::new(WorkspaceAgentProfile::new(
            ConfiguredWorkspaces::from_current_dir()?,
        )),
    )?;
    let session = "ahp-session:/client-isolation";
    let stale_chat = "ahp-chat:/stale";
    let kept_chat = "ahp-chat:/kept";
    store.create_session(CreateSession {
        session: session.into(),
        default_chat: stale_chat.into(),
        provider: "test".into(),
        workspace: "file:///workspace".into(),
        model_selection: ModelCatalog::single("test").default_selection,
    })?;
    store.mark_session_ready(MarkSessionReady {
        session: session.into(),
    })?;
    store.create_chat(CreateChat {
        session: session.into(),
        chat: kept_chat.into(),
    })?;
    store.change_session(ChangeSession {
        session: session.into(),
        origin: None,
        mutation: SessionMutation::ActiveClientSet(serde_json::from_value(json!({
            "clientId":"owner", "tools":[{"name":"removed"}, {"name":"kept"}]
        }))?),
    })?;
    let mut started = Vec::new();
    for (chat, tool_name) in [(stale_chat, "removed"), (kept_chat, "kept")] {
        let turn = store
            .start_turn(TurnStart {
                chat: chat.into(),
                origin: None,
                legacy_default_selection: None,
                action: serde_json::from_value::<ChatTurnStartedAction>(json!({
                    "turnId":tool_name, "startedAt":"2026-10-24T00:00:00.000Z",
                    "message":{"text":"test", "origin":{"kind":"user"}}
                }))?,
            })?
            .value;
        let presentation =
            ToolPresentation::for_turn(tool_name.into(), tool_name.into(), "{}".into(), &turn)?;
        store.start_tool(
            chat,
            &turn.turn_id,
            presentation.response_part(),
            presentation.start_action(&turn.turn_id),
        )?;
        started.push((turn, presentation));
    }
    let (kept, presentation) = &started[1];
    store.transition_tool(ToolCallTransition {
        chat: kept.chat.clone(),
        turn_id: kept.turn_id.clone(),
        tool_call_id: presentation.id.clone(),
        next_state: presentation.ready_state(ToolCallConfirmationReason::NotNeeded, None),
        actions: vec![presentation.ready_action(
            &kept.turn_id,
            Some(ToolCallConfirmationReason::NotNeeded),
            None,
        )],
        origin: None,
        allow_command: None,
    })?;
    let kept_key = ApprovalKey {
        chat: kept.chat.clone(),
        turn_id: kept.turn_id.clone(),
        tool_call_id: presentation.id.clone(),
    };
    let (sender, mut receiver) = oneshot::channel();
    host.inner.client_tools.lock().await.insert(
        kept_key.clone(),
        PendingClientTool {
            session: session.into(),
            client_id: "owner".into(),
            sender,
        },
    );
    store.change_session(ChangeSession {
        session: session.into(),
        origin: None,
        mutation: SessionMutation::ActiveClientSet(serde_json::from_value(json!({
            "clientId":"owner", "tools":[{"name":"kept"}]
        }))?),
    })?;
    let peer = host.open_peer().await;
    host.set_peer_initialized(peer.id(), "owner".into(), [session.into()])
        .await;
    let (stale, presentation) = &started[0];
    let tool = stale
        .client_tools
        .iter()
        .find(|tool| tool.definition.name == "removed")
        .ok_or("missing pinned tool")?;
    let cancellation = CancellationToken::new();
    let execution = timeout(
        Duration::from_secs(1),
        execute_client_tool(
            &host,
            stale,
            tool,
            ModelToolCall {
                id: presentation.id.clone(),
                name: "removed".into(),
                arguments: "{}".into(),
            },
            &cancellation,
        ),
    )
    .await?;
    let (_, content) = execution.map_err(|_| "stale call should return a failed tool result")?;
    let result: Value = serde_json::from_str(&content)?;
    assert_eq!(result["success"], false);
    assert!(
        matches!(tool_state(&store, stale_chat)?, ToolCallState::Completed(call) if !call.success)
    );
    assert!(matches!(
        tool_state(&store, kept_chat)?,
        ToolCallState::Running(_)
    ));
    assert!(host.inner.client_tools.lock().await.contains_key(&kept_key));
    assert!(matches!(
        receiver.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    host.shutdown_and_wait().await?;
    Ok(())
}
