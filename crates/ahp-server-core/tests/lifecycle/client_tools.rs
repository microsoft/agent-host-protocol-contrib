use super::*;
use ahp_types::state::{ResponsePart, ToolCallState};
use openai_agent_model::ModelMessage;

struct ClientModel(Mutex<Vec<ModelRequest>>);

impl AgentModel for ClientModel {
    fn default_model(&self) -> Option<&str> {
        Some("fake-model")
    }

    fn stream<'a>(&'a self, request: &'a ModelRequest) -> ModelRequestFuture<'a> {
        let first = {
            let mut requests = self.0.lock().expect("requests");
            requests.push(request.clone());
            requests.len() == 1
        };
        assert!(request.tools.iter().any(|tool| tool.name == "client_echo"));
        let events = if first {
            vec![
                ModelStreamEvent::ToolCallStarted {
                    index: 0,
                    id: "client-call".into(),
                    name: "client_echo".into(),
                },
                ModelStreamEvent::ToolCallArgumentsDelta {
                    index: 0,
                    delta: "{}".into(),
                },
                ModelStreamEvent::Completed,
            ]
        } else {
            vec![
                ModelStreamEvent::TextDelta("continued".into()),
                ModelStreamEvent::Completed,
            ]
        };
        Box::pin(async move {
            let events: ModelEventStream = Box::pin(stream::iter(events.into_iter().map(Ok)));
            Ok(events)
        })
    }
}

async fn action(peer: &mut ahp_server_core::CorePeer, kind: &str) -> Value {
    loop {
        let frame = next(peer).await;
        if frame["params"]["action"]["type"] == kind {
            return frame["params"].clone();
        }
    }
}

async fn dispatch(
    host: &CoreHost,
    peer: &ahp_server_core::CorePeer,
    chat: &str,
    seq: i64,
    action: Value,
) {
    message(host, peer.id(), json!({
        "jsonrpc":"2.0", "method":"dispatchAction", "params":{"channel":chat, "clientSeq":seq, "action":action}
    })).await;
}

async fn fixture() -> (
    CoreHost,
    Arc<MemoryHostStore>,
    Arc<ClientModel>,
    ahp_server_core::CorePeer,
    String,
) {
    let store = Arc::new(MemoryHostStore::new());
    let model = Arc::new(ClientModel(Mutex::new(Vec::new())));
    let host = CoreHost::new(
        CoreConfig::default(),
        store.clone(),
        model.clone(),
        Arc::new(FakeProfile::new()),
    )
    .expect("host");
    let mut owner = host.open_peer().await;
    let chat = initialized_session(&host, &store, &mut owner).await;
    message(&host, owner.id(), json!({"jsonrpc":"2.0", "id":4, "method":"subscribe", "params":{"channel":"ahp-session:/test"}})).await;
    response(&mut owner, 4).await;
    dispatch(&host, &owner, "ahp-session:/test", 1, json!({
        "type":"session/activeClientSet", "activeClient":{"clientId":"client", "tools":[{
            "name":"client_echo", "description":"Echo on client", "inputSchema":{"type":"object"}
        }]}
    })).await;
    assert!(action(&mut owner, "session/activeClientSet")
        .await
        .get("rejectionReason")
        .is_none());
    start(&host, owner.id(), &chat, "client-turn").await;
    action(&mut owner, "chat/toolCallReady").await;
    (host, store, model, owner, chat)
}

fn call(store: &MemoryHostStore, chat: &str) -> ToolCallState {
    let snapshot = store
        .snapshot(SnapshotRequest {
            channel: chat.into(),
            model_catalog: ModelCatalog::single("fake-model"),
            server_tools: Vec::new(),
        })
        .expect("snapshot");
    let chat = match snapshot.state {
        SnapshotState::Chat(chat) => Some(chat),
        _ => None,
    }
    .expect("chat");
    let parts = chat
        .active_turn
        .as_ref()
        .map(|turn| &turn.response_parts)
        .unwrap_or_else(|| &chat.turns.last().expect("turn").response_parts);
    parts
        .iter()
        .find_map(|part| match part {
            ResponsePart::ToolCall(tool) => Some(tool.tool_call.clone()),
            _ => None,
        })
        .expect("call")
}

#[tokio::test]
async fn owner_replaces_content_and_completes_while_non_owner_is_rejected() {
    let (host, store, model, mut owner, chat) = fixture().await;
    let mut observer = host.open_peer().await;
    message(&host, observer.id(), json!({
        "jsonrpc":"2.0", "id":1, "method":"initialize", "params":{
            "channel":"ahp-root://", "clientId":"observer", "protocolVersions":["1.0.0"], "initialSubscriptions":[chat]
        }
    })).await;
    response(&mut observer, 1).await;
    let before = call(&store, &chat);
    assert!(
        matches!(&before, ToolCallState::Running(running) if matches!(&running.contributor,
        Some(ahp_types::state::ToolCallContributor::Client(owner)) if owner.client_id == "client"))
    );
    for update in [
        json!({"type":"chat/toolCallContentChanged", "turnId":"client-turn", "toolCallId":"client-call", "content":[{"type":"text", "text":"spoofed"}]}),
        json!({"type":"chat/toolCallComplete", "turnId":"client-turn", "toolCallId":"client-call", "result":{"success":true,"pastTenseMessage":"Spoofed"}}),
    ] {
        let kind = update["type"].as_str().expect("kind").to_owned();
        dispatch(&host, &observer, &chat, 2, update).await;
        assert!(action(&mut observer, &kind).await["rejectionReason"].is_string());
        assert_eq!(call(&store, &chat), before);
    }
    for text in ["first", "replacement"] {
        let content = json!([{"type":"text", "text":text}]);
        dispatch(&host, &owner, &chat, 3, json!({"type":"chat/toolCallContentChanged", "turnId":"client-turn", "toolCallId":"client-call", "content":content})).await;
        loop {
            let echo = action(&mut owner, "chat/toolCallContentChanged").await;
            if echo.get("rejectionReason").is_none() {
                break;
            }
        }
        let running = match call(&store, &chat) {
            ToolCallState::Running(running) => Some(running),
            _ => None,
        }
        .expect("running");
        assert_eq!(
            serde_json::to_value(running.content).expect("content"),
            content
        );
    }
    assert_eq!(model.0.lock().expect("requests").len(), 1);
    dispatch(&host, &owner, &chat, 4, json!({
        "type":"chat/toolCallComplete", "turnId":"client-turn", "toolCallId":"client-call",
        "result":{"success":true,"pastTenseMessage":"Returned result","content":[{"type":"text","text":"finished"}],"structuredContent":{"text":"finished"}}
    })).await;
    loop {
        let echo = action(&mut owner, "chat/toolCallComplete").await;
        if echo.get("rejectionReason").is_none() {
            assert_eq!(echo["origin"]["clientId"], "client");
            break;
        }
    }
    action(&mut owner, "chat/turnComplete").await;
    let completed = match call(&store, &chat) {
        ToolCallState::Completed(completed) => Some(completed),
        _ => None,
    }
    .expect("completed");
    assert!(completed.success);
    assert_eq!(
        completed.structured_content.expect("structured")["text"],
        "finished"
    );
    let requests = model.0.lock().expect("requests").clone();
    assert_eq!(requests.len(), 2);
    assert!(
        matches!(requests[1].messages.last(), Some(ModelMessage::Tool { content, .. }) if content.contains("finished"))
    );
    host.shutdown_and_wait().await.expect("shutdown");
}

#[tokio::test]
async fn client_execution_is_released_on_removal_disconnect_cancel_and_shutdown() {
    for mode in ["remove", "disconnect", "cancel", "shutdown"] {
        let (host, store, model, mut owner, chat) = fixture().await;
        match mode {
            "remove" => {
                dispatch(
                    &host,
                    &owner,
                    "ahp-session:/test",
                    2,
                    json!({"type":"session/activeClientRemoved", "clientId":"client"}),
                )
                .await;
                action(&mut owner, "chat/turnComplete").await;
            }
            "disconnect" => {
                host.close_peer(owner.id()).await;
                timeout(Duration::from_secs(2), async {
                    loop {
                        if matches!(call(&store, &chat), ToolCallState::Completed(_))
                            && model.0.lock().expect("requests").len() == 2
                        {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                })
                .await
                .expect("disconnected client result");
            }
            "cancel" => {
                dispatch(
                    &host,
                    &owner,
                    &chat,
                    2,
                    json!({"type":"chat/turnCancelled", "turnId":"client-turn"}),
                )
                .await;
                action(&mut owner, "chat/turnCancelled").await;
                assert!(matches!(call(&store, &chat), ToolCallState::Cancelled(_)));
            }
            _ => {}
        }
        timeout(Duration::from_secs(2), host.shutdown_and_wait())
            .await
            .expect("no deadlock")
            .expect("shutdown");
        if mode == "remove" || mode == "disconnect" {
            assert!(
                matches!(call(&store, &chat), ToolCallState::Completed(completed) if !completed.success)
            );
            assert_eq!(model.0.lock().expect("requests").len(), 2);
        }
    }
}
