//! Observable turn streaming and cancellation probes.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use serde_json::{json, Value};

use super::advanced::{prompt_for_model, ready_chat, start_turn};
use super::{ConformanceError, Probe};

const POST_CANCEL_QUIESCENCE: Duration = Duration::from_secs(1);

pub(super) async fn streaming_lifecycle(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout_duration).await?;
    let (_session, chat) = ready_chat(&mut probe, "stream").await?;
    probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    let expected = "streamed output from the deterministic model";
    start_turn(
        &mut probe,
        &chat,
        "stream-turn",
        prompt_for_model(exact_output, expected, 4, 0),
    )
    .await?;
    probe
        .wait_action(&chat, "chat/turnStarted")
        .await
        .map_err(|error| {
            ConformanceError::Options(format!("waiting for chat/turnStarted: {error}"))
        })?;
    let response_part = probe
        .wait_action(&chat, "chat/responsePart")
        .await
        .map_err(|error| {
            ConformanceError::Options(format!("waiting for chat/responsePart: {error}"))
        })?;
    require_turn_id(&response_part, "stream-turn")?;
    if response_part
        .pointer("/params/action/part/kind")
        .and_then(Value::as_str)
        != Some("markdown")
    {
        return Err(ConformanceError::Options(
            "chat/responsePart was not a markdown part".to_owned(),
        ));
    }
    let mut response = String::new();
    loop {
        let action = probe
            .wait_action_any(&chat, &["chat/delta", "chat/turnComplete"])
            .await?;
        match action
            .pointer("/params/action/type")
            .and_then(Value::as_str)
        {
            Some("chat/delta") => {
                require_turn_id(&action, "stream-turn")?;
                let content = action
                    .pointer("/params/action/content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| ConformanceError::Options("delta omitted content".to_owned()))?;
                response.push_str(content);
            }
            Some("chat/turnComplete") => {
                require_turn_id(&action, "stream-turn")?;
                break;
            }
            _ => {
                return Err(ConformanceError::Options(
                    "unexpected action selection".to_owned(),
                ))
            }
        }
    }
    if response.is_empty() {
        return Err(ConformanceError::Options(
            "streamed turn did not produce text".to_owned(),
        ));
    }
    if exact_output && response != expected {
        return Err(ConformanceError::Options(format!(
            "deterministic response was {response:?}, expected {expected:?}"
        )));
    }
    Ok("streams responsePart, ordered deltas, and turnComplete".to_owned())
}

pub(super) async fn cancellation(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout_duration).await?;
    let (_session, chat) = ready_chat(&mut probe, "cancel").await?;
    probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    start_turn(
        &mut probe,
        &chat,
        "cancel-turn",
        prompt_for_model(exact_output, "this must be cancelled", 2, 750),
    )
    .await?;
    let response_part = probe.wait_action(&chat, "chat/responsePart").await?;
    require_turn_id(&response_part, "cancel-turn")?;
    probe
        .dispatch_action(
            &chat,
            json!({
                "type": "chat/turnCancelled",
                "turnId": "cancel-turn",
            }),
        )
        .await?;
    let cancelled_action = probe.wait_action(&chat, "chat/turnCancelled").await?;
    require_turn_id(&cancelled_action, "cancel-turn")?;
    let cancelled_sequence = Probe::action_server_sequence(&cancelled_action)?;
    probe
        .assert_no_post_cancel_activity_for_turn(
            &chat,
            "cancel-turn",
            cancelled_sequence,
            POST_CANCEL_QUIESCENCE,
        )
        .await?;
    let snapshot = probe
        .request("subscribe", json!({ "channel": chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| {
            ConformanceError::Options("cancelled chat subscription omitted a snapshot".to_owned())
        })?;
    assert_cancelled_snapshot(&snapshot, "cancel-turn")?;
    Ok(
        "cancels an in-flight turn, observes post-cancel quiescence, and persists terminal state"
            .to_owned(),
    )
}

pub(super) fn assert_cancelled_snapshot(
    snapshot: &Value,
    turn_id: &str,
) -> Result<(), ConformanceError> {
    let cancelled = snapshot
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns.iter().find(|turn| {
                turn.get("id").and_then(Value::as_str) == Some(turn_id)
                    && turn.get("state").and_then(Value::as_str) == Some("cancelled")
            })
        })
        .ok_or_else(|| {
            ConformanceError::Options(format!(
                "snapshot did not retain terminal cancelled turn {turn_id:?}"
            ))
        });
    let cancelled = cancelled?;
    if snapshot.get("activeTurn").is_some() {
        return Err(ConformanceError::Options(
            "cancellation snapshot retained an active turn".to_owned(),
        ));
    }
    if cancelled
        .get("responseParts")
        .and_then(Value::as_array)
        .is_some_and(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("toolCall"))
                .any(|tool| {
                    tool.get("status").and_then(Value::as_str) == Some("pending-confirmation")
                })
        })
    {
        return Err(ConformanceError::Options(
            "terminal cancellation snapshot retained a pending-confirmation tool".to_owned(),
        ));
    }
    Ok(())
}

fn require_turn_id(action: &Value, expected: &str) -> Result<(), ConformanceError> {
    if action
        .pointer("/params/action/turnId")
        .and_then(Value::as_str)
        == Some(expected)
    {
        return Ok(());
    }
    Err(ConformanceError::Options(format!(
        "action did not target turn {expected:?}"
    )))
}
