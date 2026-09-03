//! Durable draft replication, replay, and advanced-host restart coverage.

use std::time::Duration;

use ahp_types::actions::ActionEnvelope;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::local_stack::LocalStack;
use super::super::{ConformanceError, Probe};
use super::helpers::{
    assert_action_draft, assert_no_default_session_config, assert_rejected, assert_snapshot_draft,
    assert_snapshot_without_draft, chat_state, conditional_draft, create_session_without_config,
    initialized_probe, reasoning_selection, send_draft, send_turn, session_state, user_draft,
};

pub(in crate::runner) async fn chat_drafts(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    assert_model_less_session(stack, timeout).await?;
    let first = stack.start_host().await?;
    let target = first.websocket_url();
    reject_stale_conditional_draft(&target, timeout).await?;
    let retained = persist_and_replay(&target, timeout).await;
    let stopped = stop(first).await;
    let retained = retained?;
    stopped?;

    let restarted = stack.start_host().await?;
    let target = restarted.websocket_url();
    let verified =
        verify_restart_and_invalid_updates(&target, timeout, &retained.chat, retained.draft).await;
    let stopped = stop(restarted).await;
    verified?;
    stopped?;
    Ok(
        "persists drafts across clients, replay, and redb restart while rejecting invalid and causally stale updates"
            .to_owned(),
    )
}

struct RetainedDraft {
    chat: String,
    draft: Value,
}

async fn reject_stale_conditional_draft(
    target: &str,
    timeout: Duration,
) -> Result<(), ConformanceError> {
    let mut writer = initialized_probe(target, timeout, "stale-draft-writer").await?;
    let created = create_session_without_config(&mut writer, "stale-draft").await?;
    let mut observer = initialized_probe(target, timeout, "stale-draft-observer").await?;
    let subscribed = observer
        .request("subscribe", json!({ "channel": &created.chat }))
        .await?;
    super::helpers::require_result(&subscribed, "stale draft observer subscribe")?;

    let turn_id = format!("stale-draft-turn-{}", Uuid::new_v4());
    let turn = user_draft("Complete this turn.", Some(reasoning_selection("low")))?;
    send_turn(&mut writer, &created.chat, &turn_id, turn).await?;
    let accepted = writer
        .wait_action(&created.chat, "chat/turnStarted")
        .await?;
    if accepted
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
    {
        return Err(ConformanceError::Options(
            "remote turn used by stale-draft test was rejected".to_owned(),
        ));
    }

    let stale = conditional_draft(
        user_draft("stale local input", Some(reasoning_selection("low")))?,
        None,
    )?;
    let stale_client_seq = send_draft(&mut observer, &created.chat, Some(stale)).await?;
    let cleared = observer
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_snapshot_without_draft(
        cleared
            .pointer("/params/action")
            .ok_or_else(|| ConformanceError::Options("draft clear omitted action".to_owned()))?,
    )?;
    let rejected = observer
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_rejected(&rejected, "chat/draftChanged", stale_client_seq)?;
    assert_snapshot_without_draft(&chat_state(&mut writer, &created.chat).await?)?;
    let terminal = writer
        .wait_action_any(&created.chat, &["chat/turnComplete", "chat/error"])
        .await?;
    if terminal
        .pointer("/params/action/type")
        .and_then(Value::as_str)
        != Some("chat/turnComplete")
    {
        return Err(ConformanceError::Options(
            "remote turn used by stale-draft test did not complete".to_owned(),
        ));
    }

    observer.close().await?;
    writer.close().await
}

async fn assert_model_less_session(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<(), ConformanceError> {
    let host = stack.start_host_without_default().await?;
    let target = host.websocket_url();
    let result = async {
        let mut probe = initialized_probe(&target, timeout, "model-less-draft-session").await?;
        let resolved = probe
            .request(
                "resolveSessionConfig",
                json!({ "channel": "ahp-root://", "provider": "openai" }),
            )
            .await?;
        assert_no_default_session_config(&resolved)?;
        let created = create_session_without_config(&mut probe, "model-less-draft-session").await?;
        let session = session_state(&mut probe, &created.session).await?;
        if session
            .get("defaultChat")
            .and_then(Value::as_str)
            .filter(|chat| chat == &created.chat)
            .is_none()
        {
            return Err(ConformanceError::Options(
                "model-less createSession did not persist a default chat".to_owned(),
            ));
        }
        probe.close().await
    }
    .await;
    let stopped = stop(host).await;
    match (result, stopped) {
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

async fn persist_and_replay(
    target: &str,
    timeout: Duration,
) -> Result<RetainedDraft, ConformanceError> {
    let mut writer = initialized_probe(target, timeout, "chat-drafts-writer").await?;
    let created = create_session_without_config(&mut writer, "chat-drafts").await?;
    let mut observer = initialized_probe(target, timeout, "chat-drafts-observer").await?;
    let subscribed = observer
        .request("subscribe", json!({ "channel": &created.chat }))
        .await?;
    super::helpers::require_result(&subscribed, "observer chat subscribe")?;

    let initial = user_draft("first durable draft", Some(reasoning_selection("high")))?;
    let conditional = conditional_draft(initial.clone(), None)?;
    let initial_client_seq = send_draft(&mut writer, &created.chat, Some(conditional)).await?;
    let initial_event = writer
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&initial_event, Some(&initial), initial_client_seq)?;
    let initial_server_seq = action_sequence(&initial_event)?;
    let observed = observer
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&observed, Some(&initial), initial_client_seq)?;
    if action_sequence(&observed)? != initial_server_seq {
        return Err(ConformanceError::Options(
            "second subscribed client received a different draft action".to_owned(),
        ));
    }
    assert_snapshot_draft(&chat_state(&mut writer, &created.chat).await?, &initial)?;

    let updated = user_draft("updated durable draft", Some(reasoning_selection("high")))?;
    let update_client_seq = send_draft(&mut writer, &created.chat, Some(updated.clone())).await?;
    let updated_event = writer
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&updated_event, Some(&updated), update_client_seq)?;
    if action_sequence(&updated_event)? <= initial_server_seq {
        return Err(ConformanceError::Options(
            "updated draft did not receive a later server sequence".to_owned(),
        ));
    }
    assert_snapshot_draft(&chat_state(&mut writer, &created.chat).await?, &updated)?;

    let mut reconnected = Probe::connect(target, timeout).await?;
    let replay = reconnected
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": format!("chat-drafts-replay-{}", Uuid::new_v4()),
                "lastSeenServerSeq": initial_server_seq,
                "subscriptions": [&created.chat],
            }),
        )
        .await?;
    assert_exact_draft_replay(
        &replay,
        &created.chat,
        &updated,
        update_client_seq,
        initial_server_seq,
    )?;
    reconnected.close().await?;
    observer.close().await?;
    writer.close().await?;
    Ok(RetainedDraft {
        chat: created.chat,
        draft: updated,
    })
}

fn assert_exact_draft_replay(
    reply: &Value,
    chat: &str,
    expected: &Value,
    client_seq: i64,
    after_seq: i64,
) -> Result<(), ConformanceError> {
    let actions = super::helpers::require_result(reply, "draft reconnect")?
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("draft reconnect omitted actions".to_owned()))?;
    if actions.len() != 1 {
        return Err(ConformanceError::Options(format!(
            "draft reconnect returned {} actions instead of exactly the update",
            actions.len()
        )));
    }
    let action = &actions[0];
    let _: ActionEnvelope = serde_json::from_value(action.clone())?;
    if action.get("channel").and_then(Value::as_str) != Some(chat)
        || action.pointer("/action/type").and_then(Value::as_str) != Some("chat/draftChanged")
        || action.pointer("/action/draft") != Some(expected)
        || action.pointer("/origin/clientSeq").and_then(Value::as_i64) != Some(client_seq)
        || action
            .get("serverSeq")
            .and_then(Value::as_i64)
            .is_none_or(|seq| seq <= after_seq)
    {
        return Err(ConformanceError::Options(
            "draft reconnect did not replay exactly the later authoritative update".to_owned(),
        ));
    }
    Ok(())
}

async fn verify_restart_and_invalid_updates(
    target: &str,
    timeout: Duration,
    chat: &str,
    retained: Value,
) -> Result<(), ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "chat-drafts-restart").await?;
    assert_snapshot_draft(&chat_state(&mut probe, chat).await?, &retained)?;

    let clear_client_seq = send_draft(&mut probe, chat, None).await?;
    let cleared = probe.wait_action(chat, "chat/draftChanged").await?;
    assert_action_draft(&cleared, None, clear_client_seq)?;
    assert_snapshot_without_draft(&chat_state(&mut probe, chat).await?)?;

    let protected = user_draft(
        "draft retained after rejections",
        Some(reasoning_selection("high")),
    )?;
    let protected_client_seq = send_draft(&mut probe, chat, Some(protected.clone())).await?;
    let protected_event = probe.wait_action(chat, "chat/draftChanged").await?;
    assert_action_draft(&protected_event, Some(&protected), protected_client_seq)?;
    for draft in [
        json!({ "text": "assistant draft", "origin": { "kind": "agent" } }),
        json!({
            "text": "unknown model",
            "origin": { "kind": "user" },
            "model": { "id": "unknown-fixture-model" },
        }),
        json!({
            "text": "invalid reasoning",
            "origin": { "kind": "user" },
            "model": {
                "id": "fixture-reasoning",
                "config": { "thinkingLevel": "impossible" },
            },
        }),
    ] {
        let client_seq = send_draft(&mut probe, chat, Some(draft)).await?;
        let rejected = probe.wait_action(chat, "chat/draftChanged").await?;
        assert_rejected(&rejected, "chat/draftChanged", client_seq)?;
        assert_snapshot_draft(&chat_state(&mut probe, chat).await?, &protected)?;
    }
    probe.close().await
}

fn action_sequence(event: &Value) -> Result<i64, ConformanceError> {
    event
        .pointer("/params/serverSeq")
        .and_then(Value::as_i64)
        .ok_or_else(|| ConformanceError::Options("draft action omitted serverSeq".to_owned()))
}

async fn stop(host: advanced_workspace_host::RunningAdvancedHost) -> Result<(), ConformanceError> {
    host.shutdown()
        .await
        .map_err(|error| ConformanceError::Fixture(error.to_string()))
}
