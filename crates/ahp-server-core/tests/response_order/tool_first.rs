use std::sync::{Arc, Mutex};

use ahp_server_core::{
    CoreConfig, CoreHost, HostStore, MemoryHostStore, ModelCatalog, SnapshotRequest,
    WorkspaceAgentProfile,
};
use ahp_types::state::{ResponsePart, SnapshotState};
use openai_agent_model::ModelStreamEvent;
use serde_json::{json, Value};
use workspace_agent_tools::ConfiguredWorkspaces;

use super::{next, request, Rounds};

#[tokio::test]
async fn tool_first_round_retains_the_empty_initial_markdown_before_final_text(
) -> Result<(), Box<dyn std::error::Error>> {
    let model = Arc::new(Rounds(Mutex::new(
        vec![
            vec![
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
                ModelStreamEvent::TextDelta("final".into()),
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
            "channel": "ahp-root://", "clientId": "tool-first-client",
            "protocolVersions": ["0.8.0"], "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({ "channel": "ahp-session:/tool-first" }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/tool-first" }),
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
                    "type": "chat/turnStarted", "turnId": "tool-first",
                    "startedAt": "2026-09-02T00:00:00.000Z",
                    "message": { "text": "order", "origin": { "kind": "user" } }
                }
            }
        })
        .to_string(),
    )
    .await?;
    loop {
        let frame = next(&mut peer).await?;
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
    assert!(matches!(
        chat.turns[0].response_parts.as_slice(),
        [
            ResponsePart::Markdown(first),
            ResponsePart::ToolCall(_),
            ResponsePart::Markdown(last)
        ] if first.content.is_empty()
            && first.id == "response-tool-first"
            && last.content == "final"
            && last.id == "response-tool-first-1"
    ));
    Ok(())
}
