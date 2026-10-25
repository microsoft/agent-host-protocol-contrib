use super::*;

async fn initialize_client(host: &CoreHost, peer: &mut ahp_server_core::CorePeer, client: &str) {
    message(
        host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 1, "method": "initialize",
            "params": { "channel": "ahp-root://", "clientId": client,
                "protocolVersions": ["1.0.0"], "initialSubscriptions": ["ahp-root://"] }
        }),
    )
    .await;
    response(peer, 1).await;
}

async fn subscribe_session(host: &CoreHost, peer: &mut ahp_server_core::CorePeer) {
    message(
        host,
        peer.id(),
        json!({
            "jsonrpc": "2.0", "id": 10, "method": "subscribe",
            "params": { "channel": "ahp-session:/test" }
        }),
    )
    .await;
    response(peer, 10).await;
}

fn session_state(store: &MemoryHostStore) -> ahp_types::state::SessionState {
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: "ahp-session:/test".into(),
            model_catalog: ModelCatalog::single("fake-model"),
            server_tools: Vec::new(),
        })
        .expect("session snapshot");
    match snapshot.state {
        SnapshotState::Session(state) => Ok(*state),
        _ => Err("expected session snapshot"),
    }
    .expect("session state")
}

#[tokio::test]
async fn unsupported_version_error_is_delivered_before_peer_closes() {
    let (host, _) = host(Vec::new(), 128);
    let mut peer = host.open_peer().await;
    message(&host, peer.id(), json!({
        "jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"channel": "ahp-root://", "clientId": "unsupported", "protocolVersions": ["2.0.0"]}
    })).await;
    let error = next(&mut peer).await;
    assert_eq!(
        error["error"]["code"],
        ahp_types::errors::ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION
    );
    assert!(error["error"]["data"]["supportedVersions"].is_array());
    assert!(timeout(Duration::from_secs(1), peer.recv())
        .await
        .expect("peer closes")
        .is_none());
}

#[tokio::test]
async fn session_metadata_echoes_origin_and_notifies_root_and_other_subscribers() {
    let (host, store) = host(Vec::new(), 128);
    let mut writer = host.open_peer().await;
    initialized_session(&host, &store, &mut writer).await;
    subscribe_session(&host, &mut writer).await;
    let mut observer = host.open_peer().await;
    initialize_client(&host, &mut observer, "observer").await;
    subscribe_session(&host, &mut observer).await;
    for (seq, action, changed_field) in [
        (
            1,
            json!({"type":"session/titleChanged", "title":"Renamed"}),
            "title",
        ),
        (
            2,
            json!({"type":"session/isReadChanged", "isRead":true}),
            "status",
        ),
        (
            3,
            json!({"type":"session/isArchivedChanged", "isArchived":true}),
            "status",
        ),
    ] {
        message(
            &host,
            writer.id(),
            json!({
                "jsonrpc": "2.0", "method": "dispatchAction",
                "params": {"channel":"ahp-session:/test", "clientSeq":seq, "action":action}
            }),
        )
        .await;
        let echo = next(&mut observer).await;
        assert_eq!(echo["params"]["action"], action);
        assert_eq!(
            echo["params"]["origin"],
            json!({"clientId":"client", "clientSeq":seq})
        );
        assert!(echo["params"].get("rejectionReason").is_none());
        let notification = next(&mut observer).await;
        assert_eq!(notification["method"], "root/sessionSummaryChanged");
        let changes = notification["params"]["changes"]
            .as_object()
            .expect("partial summary");
        assert!(changes.contains_key(changed_field));
        assert!(!changes.contains_key("resource"));
        assert!(!changes.contains_key("provider"));
        assert!(!changes.contains_key("createdAt"));
    }
    let state = session_state(&store);
    let summary = store.session_summary("ahp-session:/test").expect("summary");
    assert_eq!(state.title, "Renamed");
    assert_eq!(state.status, summary.status);
    assert_eq!(store.list_sessions().expect("catalog")[0], summary);
}

#[tokio::test]
async fn eager_active_client_can_refresh_and_is_removed_on_unsubscribe_or_disconnect() {
    let (host, store) = host(Vec::new(), 128);
    let mut writer = host.open_peer().await;
    initialize_client(&host, &mut writer, "writer").await;
    message(&host, writer.id(), json!({
        "jsonrpc":"2.0", "id":2, "method":"createSession",
        "params":{"channel":"ahp-session:/test", "activeClient":{"clientId":"writer", "tools":[]}}
    })).await;
    response(&mut writer, 2).await;
    assert_eq!(session_state(&store).active_clients[0].client_id, "writer");
    subscribe_session(&host, &mut writer).await;
    let mut observer = host.open_peer().await;
    initialize_client(&host, &mut observer, "observer").await;
    subscribe_session(&host, &mut observer).await;
    message(&host, writer.id(), json!({
        "jsonrpc":"2.0", "method":"dispatchAction",
        "params":{"channel":"ahp-session:/test", "clientSeq":1, "action":{
            "type":"session/activeClientSet", "activeClient":{"clientId":"writer", "tools":[], "displayName":"Updated"}
        }}
    })).await;
    assert_eq!(
        next(&mut observer).await["params"]["action"]["type"],
        "session/activeClientSet"
    );
    assert_eq!(session_state(&store).active_clients.len(), 1);
    assert_eq!(
        session_state(&store).active_clients[0]
            .display_name
            .as_deref(),
        Some("Updated")
    );
    host.unsubscribe(writer.id(), "ahp-session:/test").await;
    let removal = next(&mut observer).await;
    assert_eq!(
        removal["params"]["action"],
        json!({"type":"session/activeClientRemoved", "clientId":"writer"})
    );
    assert!(session_state(&store).active_clients.is_empty());
    message(
        &host,
        observer.id(),
        json!({
            "jsonrpc":"2.0", "method":"dispatchAction",
            "params":{"channel":"ahp-session:/test", "clientSeq":1, "action":{
                "type":"session/activeClientSet", "activeClient":{"clientId":"observer", "tools":[]}
            }}
        }),
    )
    .await;
    assert_eq!(session_state(&store).active_clients.len(), 1);
    host.close_peer(observer.id()).await;
    assert!(session_state(&store).active_clients.is_empty());
}

#[tokio::test]
async fn eager_active_client_must_match_initialize_identity() {
    let (host, store) = host(Vec::new(), 128);
    let mut peer = host.open_peer().await;
    initialize_client(&host, &mut peer, "writer").await;
    message(&host, peer.id(), json!({
        "jsonrpc":"2.0", "id":2, "method":"createSession",
        "params":{"channel":"ahp-session:/test", "activeClient":{"clientId":"other", "tools":[]}}
    })).await;
    assert_eq!(
        next(&mut peer).await["error"]["code"],
        ahp_types::errors::json_rpc_error_codes::INVALID_PARAMS
    );
    assert!(store.list_sessions().expect("catalog").is_empty());
}

#[tokio::test]
async fn list_sessions_omits_the_terminal_cursor() {
    let (host, store) = host(Vec::new(), 128);
    let mut peer = host.open_peer().await;
    initialized_session(&host, &store, &mut peer).await;
    for (id, cursor) in [(4, None), (5, Some("offset:1"))] {
        let mut params = json!({"channel":"ahp-root://", "limit":1});
        if let Some(cursor) = cursor {
            params["cursor"] = json!(cursor);
        }
        message(
            &host,
            peer.id(),
            json!({"jsonrpc":"2.0", "id":id, "method":"listSessions", "params":params}),
        )
        .await;
        let result = next(&mut peer).await;
        assert_eq!(result["id"], id);
        assert!(result["result"].get("nextCursor").is_none());
    }
}

async fn backpressure_removes_active_client(broadcast: bool) {
    let (host, store) = host(Vec::new(), 128);
    let mut slow = host.open_peer().await;
    initialized_session(&host, &store, &mut slow).await;
    subscribe_session(&host, &mut slow).await;
    store
        .change_session(ahp_server_core::ChangeSession {
            session: "ahp-session:/test".into(),
            mutation: ahp_server_core::SessionMutation::ActiveClientSet(
                serde_json::from_value(json!({"clientId":"client", "tools":[]}))
                    .expect("active client"),
            ),
            origin: None,
        })
        .expect("register active client");
    let mut observer = host.open_peer().await;
    initialize_client(&host, &mut observer, "observer").await;
    subscribe_session(&host, &mut observer).await;
    if broadcast {
        for seq in 1..=65 {
            message(
                &host,
                observer.id(),
                json!({
                    "jsonrpc":"2.0", "method":"dispatchAction",
                    "params":{"channel":"ahp-session:/test", "clientSeq":seq,
                        "action":{"type":"session/titleChanged", "title":format!("Title {seq}")}}
                }),
            )
            .await;
            assert_eq!(
                next(&mut observer).await["params"]["action"]["type"],
                "session/titleChanged"
            );
            assert_eq!(
                next(&mut observer).await["method"],
                "root/sessionSummaryChanged"
            );
        }
    } else {
        for id in 100..=228 {
            message(
                &host,
                slow.id(),
                json!({
                    "jsonrpc":"2.0", "id":id, "method":"ping",
                    "params":{"channel":"ahp-root://"}
                }),
            )
            .await;
        }
    }
    let removal = next(&mut observer).await;
    assert_eq!(
        removal["params"]["action"],
        json!({"type":"session/activeClientRemoved", "clientId":"client"})
    );
    assert!(session_state(&store).active_clients.is_empty());
    let sequence = store.server_sequence().expect("server sequence");
    host.close_peer(slow.id()).await;
    assert_eq!(store.server_sequence().expect("server sequence"), sequence);
}

#[tokio::test]
async fn response_backpressure_removes_the_evicted_active_client_once() {
    backpressure_removes_active_client(false).await;
}

#[tokio::test]
async fn broadcast_backpressure_removes_the_evicted_active_client_once() {
    backpressure_removes_active_client(true).await;
}

#[tokio::test]
async fn session_disposal_publishes_chat_removals_before_unsubscribing_observers() {
    let (host, store) = host(Vec::new(), 128);
    let mut writer = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut writer).await;
    message(
        &host,
        writer.id(),
        json!({
            "jsonrpc": "2.0", "id": 4, "method": "createChat",
            "params": {"channel": "ahp-session:/test", "chat": "ahp-chat:/dispose-sibling"}
        }),
    )
    .await;
    response(&mut writer, 4).await;
    let mut observer = host.open_peer().await;
    initialize_client(&host, &mut observer, "dispose-observer").await;
    subscribe_session(&host, &mut observer).await;
    message(
        &host,
        writer.id(),
        json!({
            "jsonrpc": "2.0", "id": 5, "method": "disposeSession",
            "params": {"channel": "ahp-session:/test"}
        }),
    )
    .await;
    for chat in [chat.as_str(), "ahp-chat:/dispose-sibling"] {
        let event = next(&mut observer).await;
        assert_eq!(event["method"], "action");
        assert_eq!(event["params"]["channel"], "ahp-session:/test");
        assert_eq!(event["params"]["action"]["type"], "session/chatRemoved");
        assert_eq!(event["params"]["action"]["chat"], chat);
        assert!(event["params"].get("rejectionReason").is_none());
    }
    let count = next(&mut observer).await;
    assert_eq!(
        count["params"]["action"]["type"],
        "root/activeSessionsChanged"
    );
    let removed = next(&mut observer).await;
    assert_eq!(removed["method"], "root/sessionRemoved");
    assert_eq!(removed["params"]["session"], "ahp-session:/test");
    response(&mut writer, 5).await;
}
