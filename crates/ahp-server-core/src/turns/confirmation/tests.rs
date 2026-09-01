use std::sync::Arc;
use std::time::Duration;

use ahp_types::actions::{ChatTurnStartedAction, StateAction};
use ahp_types::state::{Message, SnapshotState};
use futures_util::stream;
use openai_agent_model::{AgentModel, ModelEventStream, ModelRequest, ModelRequestFuture};
use serde_json::json;
use tokio::task::yield_now;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{ConfiguredWorkspaces, ExactCommandApproval};

use super::{await_approval_for, confirm_tool, TurnOutcome};
use crate::host::CoreHost;
use crate::presentation::ToolPresentation;
use crate::store::{CreateSession, HostStore, MarkSessionReady, TurnStart};
use crate::{CoreConfig, MemoryHostStore, WorkspaceAgentProfile};

struct NeverModel;

impl AgentModel for NeverModel {
    fn model_name(&self) -> &str {
        "test"
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        Box::pin(async {
            let stream: ModelEventStream = Box::pin(stream::pending());
            Ok(stream)
        })
    }
}

fn message() -> Result<Message, String> {
    serde_json::from_value(json!({
        "text": "test", "origin": { "kind": "user" }
    }))
    .map_err(|error| error.to_string())
}

fn setup() -> Result<
    (
        CoreHost,
        Arc<MemoryHostStore>,
        crate::StartedTurn,
        ToolPresentation,
        ExactCommandApproval,
    ),
    String,
> {
    let store = Arc::new(MemoryHostStore::new());
    let host = CoreHost::new(
        CoreConfig::default(),
        store.clone(),
        Arc::new(NeverModel),
        Arc::new(WorkspaceAgentProfile::new(
            ConfiguredWorkspaces::from_current_dir().map_err(|error| error.to_string())?,
        )),
    )
    .map_err(|error| error.to_string())?;
    let session = "ahp-session:/confirmation";
    let chat = "ahp-chat:/confirmation";
    store
        .create_session(CreateSession {
            session: session.into(),
            default_chat: chat.into(),
            provider: "openai".into(),
            workspace: "file:///workspace".into(),
            model_selection: crate::ModelCatalog::single("test").default_selection,
        })
        .map_err(|error| error.to_string())?;
    store
        .mark_session_ready(MarkSessionReady {
            session: session.into(),
        })
        .map_err(|error| error.to_string())?;
    let started = store
        .start_turn(TurnStart {
            chat: chat.into(),
            action: ChatTurnStartedAction {
                turn_id: "turn".into(),
                started_at: "2026-01-01T00:00:00.000Z".into(),
                message: message()?,
                queued_message_id: None,
                meta: None,
            },
            origin: None,
            legacy_default_selection: crate::ModelCatalog::single("test").default_selection,
        })
        .map_err(|error| error.to_string())?
        .value;
    let presentation = ToolPresentation::new("tool".into(), "run_shell".into(), "{}".into())?;
    store
        .start_tool(
            &started.chat,
            &started.turn_id,
            presentation.response_part(),
            presentation.start_action(&started.turn_id),
        )
        .map_err(|error| error.to_string())?;
    let command = ExactCommandApproval::new("echo test").map_err(|error| error.to_string())?;
    Ok((host, store, started, presentation, command))
}

async fn wait_for_waiter(host: &CoreHost, started: &crate::StartedTurn) -> Result<(), String> {
    timeout(Duration::from_secs(1), async {
        loop {
            let key = crate::ApprovalKey {
                chat: started.chat.clone(),
                turn_id: started.turn_id.clone(),
                tool_call_id: "tool".into(),
            };
            if host.inner.approvals.lock().await.contains_key(&key) {
                return;
            }
            yield_now().await;
        }
    })
    .await
    .map_err(|_| "approval waiter did not register".to_owned())
}

async fn assert_later_confirmation_is_rejected(
    host: &CoreHost,
    started: &crate::StartedTurn,
) -> Result<(), String> {
    let mut peer = host.open_peer().await;
    host.set_peer_initialized(peer.id(), "client".into(), [started.chat.clone()])
        .await;
    let raw = json!({
        "type": "chat/toolCallConfirmed",
        "turnId": started.turn_id,
        "toolCallId": "tool",
        "approved": true,
        "selectedOptionId": "allow-once"
    });
    let action =
        serde_json::from_value::<StateAction>(raw.clone()).map_err(|error| error.to_string())?;
    confirm_tool(host, peer.id(), &started.chat, raw, action, None).await;
    let frame = timeout(Duration::from_secs(1), peer.recv())
        .await
        .map_err(|_| "rejection was not sent".to_owned())?
        .ok_or_else(|| "peer closed before rejection".to_owned())?;
    let frame: serde_json::Value =
        serde_json::from_str(&frame).map_err(|error| error.to_string())?;
    if frame
        .pointer("/params/rejectionReason")
        .and_then(|value| value.as_str())
        != Some("no matching shell confirmation is pending")
    {
        return Err("later confirmation was not rejected as missing".into());
    }
    host.close_peer(peer.id()).await;
    Ok(())
}

#[tokio::test]
async fn cancellation_removes_its_pending_waiter() -> Result<(), String> {
    let (host, _store, started, presentation, command) = setup()?;
    let cancellation = CancellationToken::new();
    let task_host = host.clone();
    let task_started = started.clone();
    let task_presentation = presentation.clone();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        await_approval_for(
            &task_host,
            &task_started,
            task_presentation,
            command,
            &task_cancellation,
            Duration::from_secs(60),
        )
        .await
    });
    wait_for_waiter(&host, &started).await?;
    cancellation.cancel();
    let result = task.await.map_err(|error| error.to_string())?;
    assert!(matches!(result, Err(TurnOutcome::Cancelled)));
    assert_later_confirmation_is_rejected(&host, &started).await
}

#[tokio::test]
async fn timeout_removes_its_pending_waiter() -> Result<(), String> {
    let (host, _store, started, presentation, command) = setup()?;
    let result = await_approval_for(
        &host,
        &started,
        presentation,
        command,
        &CancellationToken::new(),
        Duration::from_millis(1),
    )
    .await;
    assert!(matches!(result, Err(TurnOutcome::Failed(_))));
    assert_later_confirmation_is_rejected(&host, &started).await
}

#[test]
fn fixture_session_has_a_pending_tool() -> Result<(), String> {
    let (_host, store, started, _presentation, _command) = setup()?;
    let snapshot = store
        .snapshot(crate::SnapshotRequest {
            channel: started.chat,
            model_catalog: crate::ModelCatalog::single("test"),
            server_tools: Vec::new(),
        })
        .map_err(|error| error.to_string())?;
    if !matches!(snapshot.state, SnapshotState::Chat(_)) {
        return Err("expected a chat snapshot".into());
    }
    Ok(())
}
