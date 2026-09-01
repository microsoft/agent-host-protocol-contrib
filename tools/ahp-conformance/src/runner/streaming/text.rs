//! Black-box text-streaming assertions shared by API-selection cases.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::super::advanced::{fixture_prompt, start_turn};
use super::super::tools::helpers::require_turn;
use super::super::tools::workspace::{file_uri, workspace_chat};
use super::super::{ConformanceError, Probe};

pub(super) const TEXT_CHUNKS: usize = 5;
pub(super) const TEXT_DELAY_MS: u64 = 200;
const TEXT_RESPONSE: &str = "true upstream streaming text response";

pub(super) async fn assert_text_streaming(
    target: &str,
    timeout: Duration,
    workspace: &Path,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, suffix, &file_uri(workspace)?).await?;
    let turn_id = format!("{suffix}-text");
    let started = Instant::now();
    start_turn(
        &mut probe,
        &session.chat,
        &turn_id,
        fixture_prompt(TEXT_RESPONSE, TEXT_CHUNKS, TEXT_DELAY_MS),
    )
    .await?;
    require_turn(
        &probe.wait_action(&session.chat, "chat/turnStarted").await?,
        &turn_id,
        "chat/turnStarted",
    )?;
    require_turn(
        &probe
            .wait_action(&session.chat, "chat/responsePart")
            .await?,
        &turn_id,
        "chat/responsePart",
    )?;
    let deadline = Duration::from_millis(TEXT_DELAY_MS * TEXT_CHUNKS as u64);
    let mut response = String::new();
    let mut first_delta_at = None;
    loop {
        let event = probe
            .wait_action_any(
                &session.chat,
                &["chat/delta", "chat/turnComplete", "chat/error"],
            )
            .await?;
        let action = event
            .pointer("/params/action")
            .ok_or_else(|| ConformanceError::Options("streaming action was omitted".to_owned()))?;
        match action.get("type").and_then(Value::as_str) {
            Some("chat/delta") => {
                require_turn(&event, &turn_id, "chat/delta")?;
                let delta = action
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|delta| !delta.is_empty())
                    .ok_or_else(|| {
                        ConformanceError::Options("streaming delta was empty".to_owned())
                    })?;
                if first_delta_at.replace(started.elapsed()).is_none()
                    && started.elapsed() >= deadline
                {
                    return Err(ConformanceError::Options(
                        "first text delta arrived no earlier than the fixture's full stream delay"
                            .to_owned(),
                    ));
                }
                response.push_str(delta);
            }
            Some("chat/turnComplete") => {
                require_turn(&event, &turn_id, "chat/turnComplete")?;
                if first_delta_at.is_none() || response != TEXT_RESPONSE {
                    return Err(ConformanceError::Options(format!(
                        "streamed text was {response:?}, expected {TEXT_RESPONSE:?}"
                    )));
                }
                probe.close().await?;
                return Ok(());
            }
            Some("chat/error") => {
                return Err(ConformanceError::Options(
                    "streaming text turn ended in chat/error".to_owned(),
                ))
            }
            _ => {
                return Err(ConformanceError::Options(
                    "unexpected text stream event".to_owned(),
                ))
            }
        }
    }
}

pub(super) async fn start_text_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    response: &str,
    chunks: usize,
    delay_ms: u64,
) -> Result<(), ConformanceError> {
    start_turn(
        probe,
        chat,
        turn_id,
        fixture_prompt(response, chunks, delay_ms),
    )
    .await?;
    require_turn(
        &probe.wait_action(chat, "chat/turnStarted").await?,
        turn_id,
        "chat/turnStarted",
    )?;
    require_turn(
        &probe.wait_action(chat, "chat/responsePart").await?,
        turn_id,
        "chat/responsePart",
    )
}
