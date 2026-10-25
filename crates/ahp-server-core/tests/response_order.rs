use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ahp_server_core::{
    ActionReplay, CoreConfig, CoreHost, HostStore, MemoryHostStore, ModelCatalog, SnapshotRequest,
    WorkspaceAgentProfile,
};
use ahp_types::state::{ResponsePart, SnapshotState};
use futures_util::stream;
use openai_agent_model::{
    AgentModel, ModelEventStream, ModelRequest, ModelRequestFuture, ModelStreamEvent,
};
use serde_json::{json, Value};
use tokio::time::timeout;
use workspace_agent_tools::ConfiguredWorkspaces;

struct Rounds(Mutex<VecDeque<Vec<ModelStreamEvent>>>);

impl AgentModel for Rounds {
    fn default_model(&self) -> Option<&str> {
        Some("ordering-model")
    }

    fn stream<'a>(&'a self, _: &'a ModelRequest) -> ModelRequestFuture<'a> {
        let events = self
            .0
            .lock()
            .expect("rounds mutex")
            .pop_front()
            .unwrap_or_else(|| vec![ModelStreamEvent::Completed]);
        Box::pin(async move {
            let stream: ModelEventStream = Box::pin(stream::iter(events.into_iter().map(Ok)));
            Ok(stream)
        })
    }
}

async fn next(peer: &mut ahp_server_core::CorePeer) -> Result<Value, Box<dyn std::error::Error>> {
    let text = timeout(Duration::from_secs(2), peer.recv())
        .await?
        .ok_or("peer closed")?;
    Ok(serde_json::from_str(&text)?)
}

async fn request(
    host: &CoreHost,
    peer: &mut ahp_server_core::CorePeer,
    id: u64,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    host.handle_text(
        peer.id(),
        &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string(),
    )
    .await?;
    loop {
        let frame = next(peer).await?;
        if frame.get("id") == Some(&Value::from(id)) {
            return Ok(frame);
        }
    }
}

#[tokio::test]
async fn preserves_markdown_tool_markdown_order_across_model_rounds(
) -> Result<(), Box<dyn std::error::Error>> {
    let model = Arc::new(Rounds(Mutex::new(
        vec![
            vec![
                ModelStreamEvent::TextDelta("before".into()),
                ModelStreamEvent::ToolCallStarted {
                    index: 0,
                    id: "tool-1".into(),
                    name: "unknown_tool".into(),
                },
                ModelStreamEvent::ToolCallArgumentsDelta {
                    index: 0,
                    delta: "{}".into(),
                },
                ModelStreamEvent::Completed,
            ],
            vec![
                ModelStreamEvent::TextDelta("after".into()),
                ModelStreamEvent::Completed,
            ],
        ]
        .into(),
    )));
    let store = Arc::new(MemoryHostStore::new());
    let profile = WorkspaceAgentProfile::new(ConfiguredWorkspaces::from_current_dir()?);
    let host = CoreHost::new(
        CoreConfig::default(),
        store.clone(),
        model,
        Arc::new(profile),
    )?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "ordering-client",
            "protocolVersions": ["1.0.0"], "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({ "channel": "ahp-session:/ordering" }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/ordering" }),
    )
    .await?;
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("missing default chat")?
        .to_owned();
    request(&host, &mut peer, 4, "subscribe", json!({ "channel": chat })).await?;
    host.handle_text(
        peer.id(),
        &json!({
            "jsonrpc": "2.0", "method": "dispatchAction",
            "params": {
                "channel": chat, "clientSeq": 1,
                "action": {
                    "type": "chat/turnStarted", "turnId": "ordering",
                    "startedAt": "2026-09-02T00:00:00.000Z",
                    "message": { "text": "order", "origin": { "kind": "user" } }
                }
            }
        })
        .to_string(),
    )
    .await?;
    let mut delta_parts = Vec::new();
    loop {
        let frame = next(&mut peer).await?;
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some("chat/delta") {
            delta_parts.push((
                frame
                    .pointer("/params/action/partId")
                    .and_then(Value::as_str)
                    .ok_or("missing delta part id")?
                    .to_owned(),
                frame
                    .pointer("/params/action/content")
                    .and_then(Value::as_str)
                    .ok_or("missing delta content")?
                    .to_owned(),
            ));
        }
        if frame.pointer("/params/action/type").and_then(Value::as_str) == Some("chat/turnComplete")
        {
            break;
        }
    }
    let snapshot = store.snapshot(SnapshotRequest {
        channel: chat,
        model_catalog: ModelCatalog::single("ordering-model"),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    assert_eq!(
        delta_parts,
        vec![
            ("response-ordering".into(), "before".into()),
            ("response-ordering-1".into(), "after".into()),
        ]
    );
    let parts = &chat.turns[0].response_parts;
    assert!(matches!(
        parts.as_slice(),
        [
            ResponsePart::Markdown(first),
            ResponsePart::ToolCall(_),
            ResponsePart::Markdown(last)
        ] if first.content == "before"
            && first.id == "response-ordering"
            && last.content == "after"
            && last.id == "response-ordering-1"
    ));
    let ActionReplay::Replay(replay) = store.replay_since(0)? else {
        return Err("expected replay".into());
    };
    let ordering = replay
        .iter()
        .filter_map(|envelope| match envelope.action {
            ahp_types::actions::StateAction::ChatResponsePart(_) => Some("markdown"),
            ahp_types::actions::StateAction::ChatToolCallStart(_) => Some("tool"),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(ordering, vec!["markdown", "tool", "markdown"]);
    Ok(())
}

#[path = "response_order/tool_first.rs"]
mod tool_first;
