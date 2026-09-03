//! Draft clearing and retention at the chat-turn transaction boundary.

use std::time::Duration;

use ahp_types::actions::ActionEnvelope;
use serde_json::{json, Value};

use super::super::advanced::fixture_prompt;
use super::super::{ConformanceError, Probe};
use super::helpers::{
    assert_action_draft, assert_rejected, assert_snapshot_draft, assert_snapshot_without_draft,
    chat_state, create_session_without_config, initialized_probe, reasoning_selection, send_draft,
    send_turn, user_draft,
};

const RESPONSE: &str = "draft message completed";

pub(in crate::runner) async fn draft_send_lifecycle(
    target: &str,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "draft-send-lifecycle").await?;
    let created = create_session_without_config(&mut probe, "draft-send-lifecycle").await?;
    let selection = reasoning_selection("high");
    let draft = user_draft(&fixture_prompt(RESPONSE, 2, 0), Some(selection.clone()))?;
    let draft_client_seq = send_draft(&mut probe, &created.chat, Some(draft.clone())).await?;
    let saved = probe
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&saved, Some(&draft), draft_client_seq)?;

    let started_client_seq = send_turn(
        &mut probe,
        &created.chat,
        "draft-send-success",
        draft.clone(),
    )
    .await?;
    let started = probe.wait_action(&created.chat, "chat/turnStarted").await?;
    assert_started(&started, "draft-send-success", &draft, started_client_seq)?;
    let started_seq = action_sequence(&started)?;
    let cleared = probe
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&cleared, None, started_client_seq)?;
    let cleared_seq = action_sequence(&cleared)?;
    if cleared_seq <= started_seq {
        return Err(ConformanceError::Options(
            "draft clear was not journaled after chat/turnStarted".to_owned(),
        ));
    }
    let response_part = probe
        .wait_action(&created.chat, "chat/responsePart")
        .await?;
    assert_initial_response_part(&response_part, "draft-send-success", cleared_seq)?;
    await_completed_turn(&mut probe, &created.chat, "draft-send-success").await?;

    let state = chat_state(&mut probe, &created.chat).await?;
    assert_snapshot_without_draft(&state)?;
    assert_historical_model(&state, "draft-send-success", &selection)?;

    let retained = user_draft("keep this draft on turn rejection", Some(selection))?;
    let retained_client_seq = send_draft(&mut probe, &created.chat, Some(retained.clone())).await?;
    let saved = probe
        .wait_action(&created.chat, "chat/draftChanged")
        .await?;
    assert_action_draft(&saved, Some(&retained), retained_client_seq)?;
    let rejected_message = json!({
        "text": "turn with unknown selected model",
        "origin": { "kind": "user" },
        "model": { "id": "unknown-fixture-model" },
    });
    let rejected_client_seq = send_turn(
        &mut probe,
        &created.chat,
        "draft-send-rejected",
        rejected_message,
    )
    .await?;
    let rejected = probe.wait_action(&created.chat, "chat/turnStarted").await?;
    assert_rejected(&rejected, "chat/turnStarted", rejected_client_seq)?;
    assert_snapshot_draft(&chat_state(&mut probe, &created.chat).await?, &retained)?;
    probe.close().await?;
    Ok(
        "atomically clears accepted drafts with turn start and retains them after rejected turns"
            .to_owned(),
    )
}

fn assert_started(
    event: &Value,
    turn_id: &str,
    message: &Value,
    client_seq: i64,
) -> Result<(), ConformanceError> {
    let _: ActionEnvelope = serde_json::from_value(
        event
            .get("params")
            .cloned()
            .ok_or_else(|| ConformanceError::Options("turn action omitted params".to_owned()))?,
    )?;
    if event
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
        || event
            .pointer("/params/origin/clientSeq")
            .and_then(Value::as_i64)
            != Some(client_seq)
        || event
            .pointer("/params/action/turnId")
            .and_then(Value::as_str)
            != Some(turn_id)
        || event.pointer("/params/action/message") != Some(message)
    {
        return Err(ConformanceError::Options(
            "accepted turn did not retain the draft message and origin".to_owned(),
        ));
    }
    Ok(())
}

fn assert_initial_response_part(
    event: &Value,
    turn_id: &str,
    after_sequence: i64,
) -> Result<(), ConformanceError> {
    if event
        .pointer("/params/action/turnId")
        .and_then(Value::as_str)
        != Some(turn_id)
        || event
            .pointer("/params/action/part/kind")
            .and_then(Value::as_str)
            != Some("markdown")
        || event
            .pointer("/params/action/part/id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        || action_sequence(event)? <= after_sequence
    {
        return Err(ConformanceError::Options(
            "accepted turn did not journal an ordered initial markdown response part".to_owned(),
        ));
    }
    Ok(())
}

async fn await_completed_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
) -> Result<(), ConformanceError> {
    let mut saw_delta = false;
    loop {
        let event = probe
            .wait_action_any(chat, &["chat/delta", "chat/turnComplete", "chat/error"])
            .await?;
        match event.pointer("/params/action/type").and_then(Value::as_str) {
            Some("chat/delta") => {
                if event
                    .pointer("/params/action/turnId")
                    .and_then(Value::as_str)
                    != Some(turn_id)
                    || event
                        .pointer("/params/action/content")
                        .and_then(Value::as_str)
                        .is_none_or(str::is_empty)
                {
                    return Err(ConformanceError::Options(
                        "draft turn emitted an invalid text delta".to_owned(),
                    ));
                }
                saw_delta = true;
            }
            Some("chat/turnComplete") if saw_delta => {
                if event
                    .pointer("/params/action/turnId")
                    .and_then(Value::as_str)
                    == Some(turn_id)
                {
                    return Ok(());
                }
            }
            Some("chat/error") => {
                return Err(ConformanceError::Options(
                    "draft turn ended with chat/error".to_owned(),
                ))
            }
            _ => {
                return Err(ConformanceError::Options(
                    "draft turn completed before a text response lifecycle".to_owned(),
                ))
            }
        }
    }
}

fn assert_historical_model(
    state: &Value,
    turn_id: &str,
    expected: &Value,
) -> Result<(), ConformanceError> {
    let model = state
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        })
        .and_then(|turn| turn.pointer("/message/model"));
    if model == Some(expected) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "completed draft turn retained model {model:?}, expected {expected}"
        )))
    }
}

fn action_sequence(event: &Value) -> Result<i64, ConformanceError> {
    event
        .pointer("/params/serverSeq")
        .and_then(Value::as_i64)
        .ok_or_else(|| ConformanceError::Options("action omitted serverSeq".to_owned()))
}
