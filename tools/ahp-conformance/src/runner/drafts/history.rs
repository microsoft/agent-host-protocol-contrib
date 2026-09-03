//! Prior-turn context supplied to each model request.

use std::time::Duration;

use serde_json::Value;

use super::super::ConformanceError;
use super::helpers::{create_session_without_config, initialized_probe, send_turn, user_draft};

const FIRST_RESPONSE: &str = "history sentinel response";

pub(in crate::runner) async fn conversation_history(
    target: &str,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "conversation-history").await?;
    let created = create_session_without_config(&mut probe, "conversation-history").await?;

    let first_prompt =
        format!("[AHP-TEST v1]\nRespond exactly with JSON: {FIRST_RESPONSE:?}\n[/AHP-TEST]");
    let first =
        complete_text_turn(&mut probe, &created.chat, "history-first", &first_prompt).await?;
    if first != FIRST_RESPONSE {
        return Err(ConformanceError::Options(format!(
            "history setup turn returned {first:?}, expected {FIRST_RESPONSE:?}"
        )));
    }

    let second = complete_text_turn(
        &mut probe,
        &created.chat,
        "history-second",
        "[AHP-TEST v3]\nRepeat the previous assistant response exactly.\n[/AHP-TEST]",
    )
    .await?;
    if second != FIRST_RESPONSE {
        return Err(ConformanceError::Options(format!(
            "model received insufficient chat history and returned {second:?}"
        )));
    }
    probe.close().await?;
    Ok("supplies prior user and assistant turns to the next model request".to_owned())
}

async fn complete_text_turn(
    probe: &mut super::super::Probe,
    chat: &str,
    turn_id: &str,
    prompt: &str,
) -> Result<String, ConformanceError> {
    let message = user_draft(prompt, None)?;
    send_turn(probe, chat, turn_id, message).await?;
    let started = probe.wait_action(chat, "chat/turnStarted").await?;
    require_turn(&started, turn_id, "chat/turnStarted")?;
    if started
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
    {
        return Err(ConformanceError::Options(format!(
            "history turn {turn_id:?} was rejected"
        )));
    }

    let mut text = String::new();
    loop {
        let event = probe
            .wait_action_any(chat, &["chat/delta", "chat/turnComplete", "chat/error"])
            .await?;
        require_turn(&event, turn_id, "history turn event")?;
        match event.pointer("/params/action/type").and_then(Value::as_str) {
            Some("chat/delta") => {
                let chunk = event
                    .pointer("/params/action/content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConformanceError::Options(
                            "history turn emitted a non-text delta".to_owned(),
                        )
                    })?;
                text.push_str(chunk);
            }
            Some("chat/turnComplete") => return Ok(text),
            Some("chat/error") => {
                return Err(ConformanceError::Options(format!(
                    "history turn {turn_id:?} ended with chat/error"
                )))
            }
            _ => {}
        }
    }
}

fn require_turn(event: &Value, turn_id: &str, operation: &str) -> Result<(), ConformanceError> {
    if event
        .pointer("/params/action/turnId")
        .and_then(Value::as_str)
        == Some(turn_id)
    {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "{operation} did not target turn {turn_id:?}"
        )))
    }
}
