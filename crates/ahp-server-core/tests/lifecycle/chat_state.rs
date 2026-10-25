use super::*;

#[tokio::test]
async fn create_chat_initial_message_starts_a_user_turn_before_the_response() {
    let (host, store) = host(vec![Round::Pending], 16);
    let mut peer = host.open_peer().await;
    initialized_session(&host, &store, &mut peer).await;
    message(
        &host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "createChat",
            "params": {
                "channel": "ahp-session:/test", "chat": "ahp-chat:/initial",
                "initialMessage": { "text": "initial prompt", "origin": { "kind": "user" } }
            }
        }),
    )
    .await;
    response(&mut peer, 4).await;
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: "ahp-chat:/initial".into(),
            model_catalog: ModelCatalog::single("fake-model"),
            server_tools: Vec::new(),
        })
        .expect("initial chat snapshot");
    let state = serde_json::to_value(snapshot.state).expect("chat JSON");
    assert_eq!(state["activeTurn"]["message"]["text"], "initial prompt");
    host.shutdown_and_wait().await.expect("shutdown");
}

#[tokio::test]
async fn idle_queued_input_emits_removal_then_a_turn_with_the_pending_id() {
    let (host, store) = host(vec![Round::Events(vec![ModelStreamEvent::Completed])], 16);
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    message(
        &host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "method": "dispatchAction",
            "params": { "channel": chat, "clientSeq": 4, "action": {
                "type": "chat/pendingMessageSet", "id": "queued", "kind": "queued",
                "message": { "text": "next prompt", "origin": { "kind": "user" } }
            }}
        }),
    )
    .await;
    let mut removal_sequence = None;
    let mut started = false;
    loop {
        let frame = next(&mut peer).await;
        match frame.pointer("/params/action/type").and_then(Value::as_str) {
            Some("chat/pendingMessageRemoved") => {
                assert_eq!(frame.pointer("/params/action/id"), Some(&json!("queued")));
                removal_sequence = frame.pointer("/params/serverSeq").and_then(Value::as_u64);
            }
            Some("chat/turnStarted") => {
                assert_eq!(
                    frame.pointer("/params/action/queuedMessageId"),
                    Some(&json!("queued"))
                );
                let removed = removal_sequence.expect("pending removal precedes turn start");
                assert!(
                    removed
                        < frame
                            .pointer("/params/serverSeq")
                            .and_then(Value::as_u64)
                            .expect("start sequence")
                );
                started = true;
            }
            Some("chat/turnComplete") => break,
            _ => {}
        }
    }
    assert!(started);
    assert!(store
        .pending_messages(&chat)
        .expect("pending input")
        .entries
        .is_empty());
    host.shutdown_and_wait().await.expect("shutdown");
}

#[tokio::test]
async fn snapshots_keep_only_the_latest_steering_message_outside_the_queue() {
    let (host, store) = host(Vec::new(), 16);
    let mut peer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut peer).await;
    for (sequence, id) in [(4, "first"), (5, "second")] {
        message(
            &host,
            peer.id(),
            json!({
                "jsonrpc": "2.0", "method": "dispatchAction",
                "params": { "channel": chat, "clientSeq": sequence, "action": {
                    "type": "chat/pendingMessageSet", "id": id, "kind": "steering",
                    "message": { "text": id, "origin": { "kind": "user" } }
                }}
            }),
        )
        .await;
    }
    message(
        &host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 6, "method": "subscribe", "params": { "channel": chat }
        }),
    )
    .await;
    loop {
        let frame = next(&mut peer).await;
        if frame.get("id") == Some(&json!(6)) {
            let state = &frame["result"]["snapshot"]["state"];
            assert!(state.get("queuedMessages").is_none());
            assert_eq!(state["steeringMessage"]["id"], "second");
            assert!(state["steeringMessage"].get("kind").is_none());
            break;
        }
    }
    host.shutdown_and_wait().await.expect("shutdown");
}
