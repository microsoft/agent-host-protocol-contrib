use super::*;

#[tokio::test]
async fn persists_config_changes_and_records_effective_turn_models(
) -> Result<(), Box<dyn std::error::Error>> {
    let (host, store, model) = host()?;
    let mut peer = host.open_peer().await;
    request(
        &host,
        &mut peer,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://", "clientId": "client", "protocolVersions": ["0.8.0"],
            "initialSubscriptions": ["ahp-root://"]
        }),
    )
    .await?;
    request(
        &host,
        &mut peer,
        2,
        "createSession",
        json!({
            "channel": "ahp-session:/models",
            "config": { "model": { "id": "beta", "config": { "thinkingLevel": "medium" } } }
        }),
    )
    .await?;
    let session = request(
        &host,
        &mut peer,
        3,
        "subscribe",
        json!({ "channel": "ahp-session:/models" }),
    )
    .await?;
    let chat = session["result"]["snapshot"]["state"]["defaultChat"]
        .as_str()
        .ok_or("missing default chat")?
        .to_owned();
    assert_eq!(
        session.pointer("/result/snapshot/state/config/values/model/id"),
        Some(&json!("beta"))
    );
    request(&host, &mut peer, 4, "subscribe", json!({ "channel": chat })).await?;

    notify(
        &host,
        &peer,
        &chat,
        5,
        json!({
            "type": "chat/turnStarted", "turnId": "override",
            "startedAt": "2026-09-01T00:00:00.000Z",
            "message": {
                "text": "first", "origin": { "kind": "user" },
                "model": { "id": "alpha", "config": { "thinkingLevel": "medium" } }
            }
        }),
    )
    .await?;
    wait_for_requests(&model, 1).await?;

    notify(
        &host,
        &peer,
        "ahp-session:/models",
        6,
        json!({
            "type": "session/configChanged",
            "config": { "model": { "id": "beta", "config": { "thinkingLevel": "high" } } }
        }),
    )
    .await?;
    let changed = action(&mut peer, "session/configChanged").await?;
    assert_eq!(changed.pointer("/params/origin/clientSeq"), Some(&json!(6)));
    assert_eq!(
        changed.pointer("/params/action/config/model/id"),
        Some(&json!("beta"))
    );

    notify(
        &host,
        &peer,
        "ahp-session:/models",
        7,
        json!({
            "type": "session/configChanged",
            "config": { "model": { "id": "beta", "config": { "thinkingLevel": "invalid" } } }
        }),
    )
    .await?;
    let rejected = action(&mut peer, "session/configChanged").await?;
    assert_eq!(
        rejected.pointer("/params/origin/clientSeq"),
        Some(&json!(7))
    );
    assert!(rejected.pointer("/params/rejectionReason").is_some());

    notify(
        &host,
        &peer,
        &chat,
        8,
        json!({
            "type": "chat/turnStarted", "turnId": "default",
            "startedAt": "2026-09-01T00:01:00.000Z",
            "message": { "text": "second", "origin": { "kind": "user" } }
        }),
    )
    .await?;
    wait_for_requests(&model, 2).await?;
    host.shutdown_and_wait().await?;

    let requests = model.requests();
    let first_selection = requests[0]
        .model_selection
        .as_ref()
        .ok_or("first request lacks model selection")?;
    assert_eq!(first_selection.id, "alpha");
    assert_eq!(
        first_selection.reasoning_effort,
        Some(ReasoningEffort::Medium)
    );
    let second_selection = requests[1]
        .model_selection
        .as_ref()
        .ok_or("second request lacks model selection")?;
    assert_eq!(second_selection.id, "beta");
    assert_eq!(
        second_selection.reasoning_effort,
        Some(ReasoningEffort::High)
    );
    let snapshot = store.snapshot(SnapshotRequest {
        channel: chat,
        model_catalog: catalog(),
        server_tools: Vec::new(),
    })?;
    let ahp_types::state::SnapshotState::Chat(chat) = snapshot.state else {
        return Err("expected chat snapshot".into());
    };
    assert_eq!(
        chat.turns[0]
            .message
            .model
            .as_ref()
            .map(|model| model.id.as_str()),
        Some("alpha")
    );
    assert_eq!(
        chat.turns[1]
            .message
            .model
            .as_ref()
            .map(|model| model.id.as_str()),
        Some("beta")
    );
    let ahp_server_core::ActionReplay::Replay(replay) = store.replay_since(0)? else {
        return Err("expected replayable actions".into());
    };
    assert!(replay.iter().any(|envelope| {
        matches!(
            envelope.action,
            ahp_types::actions::StateAction::SessionConfigChanged(_)
        ) && envelope.rejection_reason.is_none()
    }));
    assert!(replay.iter().any(|envelope| {
        matches!(
            envelope.action,
            ahp_types::actions::StateAction::SessionConfigChanged(_)
        ) && envelope.rejection_reason.is_some()
    }));
    Ok(())
}
