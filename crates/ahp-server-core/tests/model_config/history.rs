use super::*;
use openai_agent_model::ModelMessage;

struct HistoryModel {
    requests: Mutex<Vec<ModelRequest>>,
    response_chain_scope: Option<String>,
}

impl HistoryModel {
    fn requests(&self) -> Vec<ModelRequest> {
        self.requests.lock().expect("model recorder mutex").clone()
    }
}

impl AgentModel for HistoryModel {
    fn default_model(&self) -> Option<&str> {
        Some("alpha")
    }

    fn response_chain_scope(&self) -> Option<&str> {
        self.response_chain_scope.as_deref()
    }

    fn stream<'a>(&'a self, request: &'a ModelRequest) -> ModelRequestFuture<'a> {
        let request_number = {
            let mut requests = self.requests.lock().expect("model recorder mutex");
            requests.push(request.clone());
            requests.len()
        };
        let chained = self.response_chain_scope.is_some();
        Box::pin(async move {
            let mut events = Vec::new();
            if chained {
                events.push(Ok(ModelStreamEvent::ResponseStarted {
                    id: format!("resp-{request_number}"),
                }));
            }
            if request_number == 1 {
                events.extend([
                    Ok(ModelStreamEvent::ToolCallStarted {
                        index: 0,
                        id: "history-tool".into(),
                        name: "unknown_tool".into(),
                    }),
                    Ok(ModelStreamEvent::ToolCallArgumentsDelta {
                        index: 0,
                        delta: "{}".into(),
                    }),
                ]);
            } else if request_number == 2 {
                events.push(Ok(ModelStreamEvent::TextDelta("first answer".into())));
            }
            events.push(Ok(ModelStreamEvent::Completed));
            let stream: ModelEventStream = Box::pin(stream::iter(events));
            Ok(stream)
        })
    }
}

#[tokio::test]
async fn supplies_completed_chat_turns_to_the_next_model_request(
) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(MemoryHostStore::new());
    let model = Arc::new(HistoryModel {
        requests: Mutex::new(Vec::new()),
        response_chain_scope: Some("https://api.openai.com/v1/responses".into()),
    });
    let profile = WorkspaceAgentProfile::new(ConfiguredWorkspaces::from_current_dir()?);
    let host = CoreHost::with_model_catalog(
        CoreConfig::default(),
        catalog(),
        store,
        model.clone(),
        Arc::new(profile),
    )?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "history-client",
            "protocolVersions": ["1.0.0"], "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({ "channel": "ahp-session:/history" }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/history" }),
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
            "type": "chat/turnStarted", "turnId": "first",
            "startedAt": "2026-09-02T00:00:00.000Z",
            "message": { "text": "first question", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    action(&mut peer, "chat/turnComplete").await?;
    notify(
        &host,
        &peer,
        &chat,
        6,
        json!({
            "type": "chat/turnStarted", "turnId": "second",
            "startedAt": "2026-09-02T00:01:00.000Z",
            "message": { "text": "what was your last response?", "origin": { "kind": "user" } }
        }),
    )
    .await?;

    for _ in 0..100 {
        if model.requests().len() == 3 {
            break;
        }
        yield_now().await;
    }
    host.shutdown_and_wait().await?;

    let requests = model.requests();
    assert_eq!(requests.len(), 3);
    assert!(requests.iter().all(|request| request.store_response));
    assert_eq!(
        requests[1].response_continuation,
        Some(openai_agent_model::ResponseContinuation {
            previous_response_id: "resp-1".into(),
            input_start: 2,
        })
    );
    assert!(matches!(
        requests[1].messages.get(2),
        Some(ModelMessage::Tool {
            tool_call_id,
            ..
        }) if tool_call_id == "history-tool"
    ));
    assert_eq!(
        requests[2].messages[..2],
        [
            ModelMessage::User {
                content: "first question".into(),
            },
            ModelMessage::Assistant {
                content: Some("first answer".into()),
                tool_calls: Vec::new(),
            },
        ]
    );
    assert_eq!(
        requests[2].response_continuation,
        Some(openai_agent_model::ResponseContinuation {
            previous_response_id: "resp-2".into(),
            input_start: 2,
        })
    );
    let Some(ModelMessage::User { content }) = requests[2].messages.last() else {
        return Err("second request did not end with the current user message".into());
    };
    assert!(content.ends_with("\n\nwhat was your last response?"));
    Ok(())
}
