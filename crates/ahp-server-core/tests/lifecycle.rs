use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ahp_server_core::{
    AgentProfile, CoreConfig, CoreHost, HostStore, MemoryHostStore, ModelCatalog, ProfileError,
    ProfileFuture, SnapshotRequest, WorkspaceAgentProfile,
};
use ahp_types::common::Uri;
use ahp_types::state::SnapshotState;
use futures_util::stream;
use openai_agent_model::{
    AgentModel, ModelEventStream, ModelRequest, ModelRequestFuture, ModelStreamEvent,
};
use serde_json::{json, Value};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{
    ConfiguredWorkspaces, ExactCommandApproval, ExecutionOutcome, PreparedTool, SelectedWorkspace,
    ToolError, ToolExecution, ToolInvocation,
};

enum Round {
    Events(Vec<ModelStreamEvent>),
    Pending,
}

struct FakeModel {
    rounds: Mutex<VecDeque<Round>>,
}

impl FakeModel {
    fn new(rounds: Vec<Round>) -> Self {
        Self {
            rounds: Mutex::new(rounds.into()),
        }
    }
}

impl AgentModel for FakeModel {
    fn default_model(&self) -> Option<&str> {
        Some("fake-model")
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        let round = self
            .rounds
            .lock()
            .expect("test fake mutex")
            .pop_front()
            .unwrap_or(Round::Events(vec![ModelStreamEvent::Completed]));
        Box::pin(async move {
            let events: ModelEventStream = match round {
                Round::Events(events) => Box::pin(stream::iter(events.into_iter().map(Ok))),
                Round::Pending => Box::pin(stream::pending()),
            };
            Ok(events)
        })
    }
}

struct FakeProfile {
    inner: WorkspaceAgentProfile,
}

impl FakeProfile {
    fn new() -> Self {
        Self {
            inner: WorkspaceAgentProfile::new(
                ConfiguredWorkspaces::from_current_dir().expect("current directory is a workspace"),
            ),
        }
    }
}

impl AgentProfile for FakeProfile {
    fn select_workspace(
        &self,
        requested: Option<Vec<Uri>>,
    ) -> Result<SelectedWorkspace, ProfileError> {
        self.inner.select_workspace(requested)
    }

    fn server_tools(&self) -> Vec<ahp_types::state::ToolDefinition> {
        self.inner.server_tools()
    }

    fn model_tools(&self) -> Vec<openai_agent_model::ModelToolDefinition> {
        self.inner.model_tools()
    }

    fn preflight(
        &self,
        workspace: &SelectedWorkspace,
        invocation: ToolInvocation,
    ) -> Result<PreparedTool, ToolError> {
        self.inner.preflight(workspace, invocation)
    }

    fn exact_command_approval(&self, prepared: &PreparedTool) -> Option<ExactCommandApproval> {
        self.inner.exact_command_approval(prepared)
    }

    fn execute<'a>(
        &'a self,
        _: &'a SelectedWorkspace,
        _: PreparedTool,
        _: &'a CancellationToken,
        _: &'a CancellationToken,
    ) -> ProfileFuture<'a> {
        Box::pin(async {
            ExecutionOutcome::Completed(ToolExecution::succeeded("Ran test tool", "test output"))
        })
    }
}

fn host(rounds: Vec<Round>, max_delta_scalars: usize) -> (CoreHost, Arc<MemoryHostStore>) {
    let store = Arc::new(MemoryHostStore::new());
    let host = CoreHost::new(
        CoreConfig {
            server_name: "test-host".into(),
            max_delta_scalars,
            outbound_queue_capacity: 128,
        },
        store.clone(),
        Arc::new(FakeModel::new(rounds)),
        Arc::new(FakeProfile::new()),
    )
    .expect("valid test configuration");
    (host, store)
}

async fn message(host: &CoreHost, peer: u64, value: Value) {
    host.handle_text(peer, &value.to_string())
        .await
        .expect("frame processing");
}

async fn next(peer: &mut ahp_server_core::CorePeer) -> Value {
    let text = timeout(Duration::from_secs(2), peer.recv())
        .await
        .expect("outbound frame timeout")
        .expect("outbound peer closed");
    serde_json::from_str(&text).expect("valid JSON-RPC frame")
}

async fn response(peer: &mut ahp_server_core::CorePeer, id: u64) {
    loop {
        if next(peer).await.get("id") == Some(&Value::from(id)) {
            return;
        }
    }
}

#[tokio::test]
async fn initialize_negotiates_compatible_versions_and_rejects_malformed_offers() {
    let (host, _) = host(Vec::new(), 128);
    for (versions, expected) in [
        (json!(["0.9.0", "1.0.0"]), Some("1.0.0")),
        (json!(["1.0.0", "1.2.3", "1.1.9", "2.0.0"]), Some("1.2.3")),
        (json!(["0.9.7", "0.10.0"]), Some("0.9.7")),
        (json!(["1.0.0", "invalid"]), None),
        (json!(["01.0.0"]), None),
        (json!(["1.0.0", 1]), None),
    ] {
        let mut peer = host.open_peer().await;
        message(
            &host,
            peer.id(),
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {
                    "channel": "ahp-root://", "clientId": "negotiation-test",
                    "protocolVersions": versions,
                }
            }),
        )
        .await;
        let reply = next(&mut peer).await;
        if let Some(version) = expected {
            assert_eq!(reply["result"]["protocolVersion"], version);
        } else {
            assert_eq!(
                reply["error"]["code"],
                ahp_types::errors::json_rpc_error_codes::INVALID_PARAMS
            );
        }
    }
}

async fn initialized_session(
    host: &CoreHost,
    store: &MemoryHostStore,
    peer: &mut ahp_server_core::CorePeer,
) -> String {
    message(host, peer.id(), json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": { "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["1.0.0"], "initialSubscriptions": ["ahp-root://"] }
    })).await;
    response(peer, 1).await;
    message(
        host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 2, "method": "createSession",
            "params": { "channel": "ahp-session:/test" }
        }),
    )
    .await;
    response(peer, 2).await;
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: "ahp-session:/test".into(),
            model_catalog: ModelCatalog::single("fake"),
            server_tools: Vec::new(),
        })
        .expect("session snapshot");
    assert!(matches!(snapshot.state, SnapshotState::Session(_)));
    let state = match snapshot.state {
        SnapshotState::Session(state) => state,
        _ => return String::new(),
    };
    let chat = state.default_chat.expect("default chat");
    message(
        host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 3, "method": "subscribe",
            "params": { "channel": chat }
        }),
    )
    .await;
    response(peer, 3).await;
    chat
}

async fn start(host: &CoreHost, peer: u64, chat: &str, turn: &str) {
    message(
        host,
        peer,
        json!({
            "jsonrpc": "2.0", "method": "dispatchAction",
            "params": { "channel": chat, "clientSeq": 1, "action": {
                "type": "chat/turnStarted", "turnId": turn, "startedAt": "2026-01-01T00:00:00.000Z",
                "message": { "text": "hello", "origin": { "kind": "user" } }
            }}
        }),
    )
    .await;
}

#[tokio::test]
async fn streams_unicode_chunks_before_turn_completion() {
    let (host, store) = host(
        vec![Round::Events(vec![
            ModelStreamEvent::TextDelta("é🙂z".into()),
            ModelStreamEvent::Completed,
        ])],
        2,
    );
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    start(&host, peer.id(), &chat, "turn-1").await;
    let mut delta_positions = Vec::new();
    let mut complete_position = None;
    let mut chunks = Vec::new();
    for position in 0..16 {
        let frame = next(&mut peer).await;
        match frame.pointer("/params/action/type").and_then(Value::as_str) {
            Some("chat/delta") => {
                delta_positions.push(position);
                chunks.push(
                    frame
                        .pointer("/params/action/content")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                );
            }
            Some("chat/turnComplete") => {
                complete_position = Some(position);
                break;
            }
            _ => {}
        }
    }
    assert_eq!(chunks, ["é🙂", "z"]);
    assert!(delta_positions
        .iter()
        .all(|position| *position < complete_position.expect("completion")));
}

#[tokio::test]
async fn duplicate_id_from_later_model_round_errors_and_clears_active_turn() {
    let call = |id: &str| {
        vec![
            ModelStreamEvent::ToolCallStarted {
                index: 0,
                id: id.into(),
                name: "replace_string".into(),
            },
            ModelStreamEvent::ToolCallArgumentsDelta {
                index: 0,
                delta: r#"{"path":"x","old_string":"a","new_string":"b"}"#.into(),
            },
            ModelStreamEvent::Completed,
        ]
    };
    let (host, store) = host(
        vec![
            Round::Events(call("duplicate")),
            Round::Events(call("duplicate")),
        ],
        10,
    );
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    start(&host, peer.id(), &chat, "turn-duplicate").await;
    let mut saw_error = false;
    for _ in 0..24 {
        let frame = next(&mut peer).await;
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some("chat/error") {
            assert_eq!(
                frame.pointer("/params/action/part/kind"),
                Some(&Value::String("error".into()))
            );
            saw_error = true;
            break;
        }
    }
    assert!(saw_error);
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: chat,
            model_catalog: ModelCatalog::single("fake"),
            server_tools: Vec::new(),
        })
        .expect("chat snapshot");
    assert!(matches!(snapshot.state, SnapshotState::Chat(_)));
    let chat = match snapshot.state {
        SnapshotState::Chat(chat) => chat,
        _ => return,
    };
    assert!(chat.active_turn.is_none());
    assert_eq!(
        chat.turns.last().expect("terminal turn").state,
        ahp_types::state::TurnState::Error
    );
    assert!(matches!(
        chat.turns
            .last()
            .expect("terminal turn")
            .response_parts
            .last(),
        Some(ahp_types::state::ResponsePart::Error(_))
    ));
}

#[tokio::test]
async fn confirmation_stores_only_exact_command_approval() {
    let (host, store) = host(
        vec![
            Round::Events(vec![
                ModelStreamEvent::ToolCallStarted {
                    index: 0,
                    id: "shell-1".into(),
                    name: "run_shell".into(),
                },
                ModelStreamEvent::ToolCallArgumentsDelta {
                    index: 0,
                    delta: r#"{"command":"echo hello"}"#.into(),
                },
                ModelStreamEvent::Completed,
            ]),
            Round::Events(vec![ModelStreamEvent::Completed]),
        ],
        10,
    );
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    start(&host, peer.id(), &chat, "turn-shell").await;
    loop {
        let frame = next(&mut peer).await;
        if frame.pointer("/params/action/type").and_then(Value::as_str)
            == Some("chat/toolCallReady")
        {
            break;
        }
    }
    message(&host, peer.id(), json!({
        "jsonrpc":"2.0", "method":"dispatchAction", "params": { "channel":chat, "clientSeq":2, "action":{
            "type":"chat/toolCallConfirmed", "turnId":"turn-shell", "toolCallId":"shell-1",
            "approved":true, "selectedOptionId":"always-allow-command"
        }}
    })).await;
    assert!(store
        .is_command_allowed(&ExactCommandApproval::new("echo hello").expect("command"))
        .expect("approval lookup"));
    assert!(!store
        .is_command_allowed(&ExactCommandApproval::new("echo goodbye").expect("command"))
        .expect("approval lookup"));
}

#[tokio::test]
async fn cancellation_terminalizes_a_quiescent_stream() {
    let (host, store) = host(vec![Round::Pending], 10);
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    start(&host, peer.id(), &chat, "turn-cancel").await;
    loop {
        let frame = next(&mut peer).await;
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some("chat/turnStarted")
        {
            break;
        }
    }
    message(&host, peer.id(), json!({
        "jsonrpc":"2.0", "method":"dispatchAction", "params": { "channel":chat, "clientSeq":2, "action":{
            "type":"chat/turnCancelled", "turnId":"turn-cancel"
        }}
    })).await;
    let mut cancelled = false;
    for _ in 0..8 {
        let frame = next(&mut peer).await;
        if frame.pointer("/params/action/type").and_then(Value::as_str)
            == Some("chat/turnCancelled")
        {
            cancelled = true;
            break;
        }
    }
    assert!(cancelled);
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: chat,
            model_catalog: ModelCatalog::single("fake"),
            server_tools: Vec::new(),
        })
        .expect("chat snapshot");
    assert!(matches!(snapshot.state, SnapshotState::Chat(_)));
    let chat = match snapshot.state {
        SnapshotState::Chat(chat) => chat,
        _ => return,
    };
    assert!(chat.active_turn.is_none());
}

#[tokio::test]
async fn shutdown_and_wait_drains_a_pending_turn_without_polling() {
    let (host, store) = host(vec![Round::Pending], 10);
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    start(&host, peer.id(), &chat, "turn-shutdown").await;
    loop {
        let frame = next(&mut peer).await;
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some("chat/turnStarted")
        {
            break;
        }
    }
    let drained = timeout(Duration::from_secs(1), host.shutdown_and_wait()).await;
    assert!(matches!(drained, Ok(Ok(()))));
}

#[path = "lifecycle/session_state.rs"]
mod session_state;

#[path = "lifecycle/chat_state.rs"]
mod chat_state;

#[path = "lifecycle/client_tools.rs"]
mod client_tools;
