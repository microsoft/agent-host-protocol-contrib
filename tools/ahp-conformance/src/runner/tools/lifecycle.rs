//! Final-response reconstruction for deterministic fixture tool turns.

use serde_json::Value;

use super::super::{ConformanceError, Probe};
use super::helpers::{action_value, require_turn};

pub(in super::super) fn fixture_tool_prompt_with_delay(
    tool: &str,
    arguments: Value,
    response: &str,
    delay_ms: u64,
) -> String {
    let mut prompt = format!(
        "[AHP-TEST v2]\nCall tool {tool} with JSON: {arguments}\nRespond exactly after tools with JSON: {}.\n[/AHP-TEST]",
        Value::String(response.to_owned())
    );
    if delay_ms > 0 {
        prompt = prompt.replace(
            "\n[/AHP-TEST]",
            &format!("\nWait {delay_ms} ms before every output chunk.\n[/AHP-TEST]"),
        );
    }
    prompt
}

pub(in super::super) async fn complete_fixture_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    expected: &str,
) -> Result<(), ConformanceError> {
    let mut response = String::new();
    loop {
        let event = probe
            .wait_action_any(chat, &["chat/delta", "chat/turnComplete", "chat/error"])
            .await?;
        let action = action_value(&event)?;
        match action.get("type").and_then(Value::as_str) {
            Some("chat/delta") => {
                require_turn(&event, turn_id, "chat/delta")?;
                response.push_str(
                    action
                        .get("content")
                        .and_then(Value::as_str)
                        .ok_or_else(|| {
                            ConformanceError::Options("delta omitted text".to_owned())
                        })?,
                );
            }
            Some("chat/turnComplete") => {
                require_turn(&event, turn_id, "chat/turnComplete")?;
                return (response == expected).then_some(()).ok_or_else(|| {
                    ConformanceError::Options(format!(
                        "fixture final text was {response:?}, expected {expected:?}"
                    ))
                });
            }
            Some("chat/error") => {
                return Err(ConformanceError::Options(format!(
                    "tool turn {turn_id:?} ended in chat/error"
                )))
            }
            _ => {
                return Err(ConformanceError::Options(
                    "unexpected terminal event".to_owned(),
                ))
            }
        }
    }
}
