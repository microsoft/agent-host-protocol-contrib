use ahp_server_core::{ActionReplay, HostStore, SnapshotRequest};
use ahp_types::state::SnapshotState;
use serde_json::{json, Value};

use super::{action, catalog, host, notify, request};

#[tokio::test]
async fn persists_valid_drafts_and_retains_them_when_a_turn_is_rejected(
) -> Result<(), Box<dyn std::error::Error>> {
    let (host, store, _) = host()?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "draft-client",
            "protocolVersions": ["0.8.0"], "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({ "channel": "ahp-session:/draft" }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/draft" }),
    )
    .await?;
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("missing default chat")?
        .to_owned();
    request(&host, &mut peer, 4, "subscribe", json!({ "channel": chat })).await?;

    send_draft(&host, &peer, &chat, 5, "first", Some("alpha")).await?;
    let set = action(&mut peer, "chat/draftChanged").await?;
    assert_eq!(set.pointer("/params/origin/clientSeq"), Some(&json!(5)));
    assert_eq!(snapshot_draft(&store, &chat)?.text, "first");
    let mut reconnecting_peer = host.open_peer().await;
    let replay = request(
        &host,
        &mut reconnecting_peer,
        1,
        "reconnect",
        json!({
            "channel": "ahp-root://", "clientId": "draft-client-two",
            "lastSeenServerSeq": 0, "subscriptions": [chat]
        }),
    )
    .await?;
    assert!(replay["result"]["actions"]
        .as_array()
        .is_some_and(|actions| {
            actions.iter().any(|action| {
                action.pointer("/action/type").and_then(Value::as_str) == Some("chat/draftChanged")
            })
        }));

    notify(
        &host,
        &peer,
        &chat,
        6,
        json!({
            "type": "chat/draftChanged",
            "draft": { "text": "invalid", "origin": { "kind": "agent" } }
        }),
    )
    .await?;
    assert!(action(&mut peer, "chat/draftChanged")
        .await?
        .pointer("/params/rejectionReason")
        .is_some());
    send_draft(&host, &peer, &chat, 7, "updated", None).await?;
    let updated = action(&mut peer, "chat/draftChanged").await?;
    assert_eq!(
        updated.pointer("/params/action/draft/text"),
        Some(&json!("updated"))
    );

    notify(
        &host,
        &peer,
        &chat,
        8,
        json!({
            "type": "chat/draftChanged",
            "draft": {
                "text": "invalid model", "origin": { "kind": "user" },
                "model": { "id": "missing" }
            }
        }),
    )
    .await?;
    assert!(action(&mut peer, "chat/draftChanged")
        .await?
        .pointer("/params/rejectionReason")
        .is_some());
    assert_eq!(snapshot_draft(&store, &chat)?.text, "updated");

    notify(
        &host,
        &peer,
        &chat,
        9,
        json!({
            "type": "chat/turnStarted", "turnId": "rejected",
            "startedAt": "2026-09-02T00:00:00.000Z",
            "message": {
                "text": "updated", "origin": { "kind": "user" },
                "model": { "id": "missing" }
            }
        }),
    )
    .await?;
    assert!(action(&mut peer, "chat/turnStarted")
        .await?
        .pointer("/params/rejectionReason")
        .is_some());
    assert_eq!(snapshot_draft(&store, &chat)?.text, "updated");

    notify(
        &host,
        &peer,
        &chat,
        10,
        json!({
            "type": "chat/turnStarted", "turnId": "accepted",
            "startedAt": "2026-09-02T00:01:00.000Z",
            "message": { "text": "updated", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    assert!(action(&mut peer, "chat/turnStarted")
        .await?
        .pointer("/params/rejectionReason")
        .is_none());
    assert_eq!(
        action(&mut peer, "chat/draftChanged")
            .await?
            .pointer("/params/action/draft"),
        None
    );
    assert!(snapshot_draft(&store, &chat).is_err());
    notify(
        &host,
        &peer,
        &chat,
        11,
        json!({
            "type": "chat/draftChanged",
            "draft": {
                "text": "stale", "origin": { "kind": "user" },
                "_meta": {
                    "com.microsoft.agent-host-protocol-samples.expectedTurnId": null
                }
            }
        }),
    )
    .await?;
    assert!(action(&mut peer, "chat/draftChanged")
        .await?
        .pointer("/params/rejectionReason")
        .is_some());
    assert!(snapshot_draft(&store, &chat).is_err());
    let ActionReplay::Replay(replay) = store.replay_since(0)? else {
        return Err("expected replay".into());
    };
    assert!(replay.windows(2).any(|actions| {
        matches!(
            actions[0].action,
            ahp_types::actions::StateAction::ChatTurnStarted(_)
        ) && matches!(
            actions[1].action,
            ahp_types::actions::StateAction::ChatDraftChanged(
                ahp_types::actions::ChatDraftChangedAction { draft: None }
            )
        )
    }));
    Ok(())
}

async fn send_draft(
    host: &ahp_server_core::CoreHost,
    peer: &ahp_server_core::CorePeer,
    chat: &str,
    client_seq: i64,
    text: &str,
    model: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let model = model.map(|id| json!({ "id": id }));
    notify(
        host,
        peer,
        chat,
        client_seq,
        json!({
            "type": "chat/draftChanged",
            "draft": {
                "text": text, "origin": { "kind": "user" },
                "model": model
            }
        }),
    )
    .await
}

fn snapshot_draft(
    store: &ahp_server_core::MemoryHostStore,
    chat: &str,
) -> Result<ahp_types::state::Message, Box<dyn std::error::Error>> {
    let snapshot = store.snapshot(SnapshotRequest {
        channel: chat.into(),
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    chat.draft.ok_or_else(|| "draft was cleared".into())
}
