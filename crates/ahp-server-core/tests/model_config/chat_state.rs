use super::*;

#[tokio::test]
async fn latest_steering_message_is_delivered_to_the_next_active_turn(
) -> Result<(), Box<dyn std::error::Error>> {
    let (host, store, model) = host()?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "steering-client", "protocolVersions": ["1.0.0"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({ "channel": "ahp-session:/steering" }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/steering" }),
    )
    .await?;
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("default chat missing")?
        .to_owned();
    request(&host, &mut peer, 4, "subscribe", json!({ "channel": chat })).await?;
    for (sequence, id) in [(5, "first"), (6, "second")] {
        notify(
            &host,
            &peer,
            &chat,
            sequence,
            json!({
                "type": "chat/pendingMessageSet", "kind": "steering", "id": id,
                "message": { "text": id, "origin": { "kind": "user" } }
            }),
        )
        .await?;
    }
    notify(&host, &peer, &chat, 7, json!({
        "type": "chat/turnStarted", "turnId": "steered", "startedAt": "2026-09-01T00:00:00.000Z",
        "message": { "text": "main prompt", "origin": { "kind": "user" } }
    })).await?;
    wait_for_requests(&model, 1).await?;
    let messages = &model.requests()[0].messages;
    assert_eq!(messages.len(), 2);
    assert!(!messages.iter().any(|message| matches!(message, openai_agent_model::ModelMessage::User { content } if content == "first")));
    assert!(
        matches!(&messages[messages.len() - 1], openai_agent_model::ModelMessage::User { content } if content == "second")
    );
    assert!(store.pending_messages(&chat)?.entries.is_empty());
    assert!(store.pending_messages(&chat)?.steering.is_none());
    notify(
        &host,
        &peer,
        &chat,
        8,
        json!({
            "type": "chat/pendingMessageSet", "kind": "steering", "id": "first",
            "message": { "text": "first", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    loop {
        let frame = next(&mut peer).await?;
        if frame.pointer("/params/origin/clientSeq") == Some(&json!(8)) {
            assert!(frame.pointer("/params/rejectionReason").is_none());
            break;
        }
    }
    assert_eq!(
        store
            .pending_messages(&chat)?
            .steering
            .map(|entry| entry.id),
        Some("first".into())
    );
    host.shutdown_and_wait().await?;
    Ok(())
}
