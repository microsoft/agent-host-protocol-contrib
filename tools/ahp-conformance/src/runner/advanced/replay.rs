//! Validation of a complete turn lifecycle in a reconnect replay.

use ahp_types::actions::ActionEnvelope;
use serde_json::Value;

use super::super::ConformanceError;

pub(super) fn validate_replayed_turn(
    actions: &[Value],
    last_seen: i64,
    channel: &str,
    turn_id: &str,
    expected_content: Option<&str>,
) -> Result<(), ConformanceError> {
    validate_sequence(actions, last_seen)?;

    let mut saw_started = false;
    let mut response_part_id = None;
    let mut terminal = None;
    let mut content = String::new();

    for envelope in actions {
        let typed: ActionEnvelope = serde_json::from_value(envelope.clone()).map_err(|error| {
            ConformanceError::Options(format!(
                "reconnect returned an invalid action envelope: {error}"
            ))
        })?;
        if typed.channel != channel
            || envelope.pointer("/action/turnId").and_then(Value::as_str) != Some(turn_id)
        {
            continue;
        }

        let action_type = envelope
            .pointer("/action/type")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ConformanceError::Options("replayed action omitted its type".to_owned())
            })?;
        if terminal.is_some() {
            return Err(ConformanceError::Options(format!(
                "replay contains {action_type} after the terminal action for turn {turn_id:?}"
            )));
        }

        match action_type {
            "chat/turnStarted" => {
                if saw_started {
                    return Err(ConformanceError::Options(format!(
                        "replay contains duplicate chat/turnStarted for turn {turn_id:?}"
                    )));
                }
                saw_started = true;
            }
            "chat/responsePart" => {
                if !saw_started || response_part_id.is_some() {
                    return Err(ConformanceError::Options(format!(
                        "chat/responsePart is duplicated or precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
                if envelope
                    .pointer("/action/part/kind")
                    .and_then(Value::as_str)
                    != Some("markdown")
                {
                    return Err(ConformanceError::Options(
                        "replayed chat/responsePart is not a markdown part".to_owned(),
                    ));
                }
                response_part_id = Some(
                    envelope
                        .pointer("/action/part/id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            ConformanceError::Options(
                                "replayed chat/responsePart omitted its part id".to_owned(),
                            )
                        })?
                        .to_owned(),
                );
            }
            "chat/delta" => {
                let part_id = response_part_id.as_deref().ok_or_else(|| {
                    ConformanceError::Options(
                        "replayed chat/delta precedes chat/responsePart".to_owned(),
                    )
                })?;
                if envelope.pointer("/action/partId").and_then(Value::as_str) != Some(part_id) {
                    return Err(ConformanceError::Options(
                        "replayed chat/delta targets a different response part".to_owned(),
                    ));
                }
                let delta = envelope
                    .pointer("/action/content")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConformanceError::Options(
                            "replayed chat/delta omitted string content".to_owned(),
                        )
                    })?;
                content.push_str(delta);
            }
            "chat/usage" => {
                if !saw_started {
                    return Err(ConformanceError::Options(format!(
                        "chat/usage precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
            }
            "chat/turnComplete" | "chat/error" => {
                if !saw_started || response_part_id.is_none() {
                    return Err(ConformanceError::Options(format!(
                        "{action_type} precedes the required opening lifecycle for turn {turn_id:?}"
                    )));
                }
                terminal = Some(action_type);
            }
            _ => {
                return Err(ConformanceError::Options(format!(
                    "unexpected replayed action {action_type:?} for turn {turn_id:?}"
                )));
            }
        }
    }

    if !saw_started || response_part_id.is_none() {
        return Err(ConformanceError::Options(format!(
            "replay omitted chat/turnStarted or chat/responsePart for turn {turn_id:?}"
        )));
    }
    let terminal = terminal.ok_or_else(|| {
        ConformanceError::Options(format!(
            "replay omitted a terminal action for turn {turn_id:?}"
        ))
    })?;
    if terminal == "chat/turnComplete" && content.is_empty() {
        return Err(ConformanceError::Options(
            "a completed replayed turn did not reconstruct any response content".to_owned(),
        ));
    }
    if let Some(expected) = expected_content {
        if terminal != "chat/turnComplete" || content != expected {
            return Err(ConformanceError::Options(format!(
                "deterministic replay content was {content:?} with {terminal}, expected {expected:?} with chat/turnComplete"
            )));
        }
    }
    Ok(())
}

fn validate_sequence(actions: &[Value], last_seen: i64) -> Result<(), ConformanceError> {
    let mut previous = last_seen;
    for action in actions {
        let sequence = action
            .get("serverSeq")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                ConformanceError::Options("replayed action omitted serverSeq".to_owned())
            })?;
        if sequence <= previous {
            return Err(ConformanceError::Options(format!(
                "replayed serverSeq {sequence} is not greater than {previous}"
            )));
        }
        previous = sequence;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::validate_sequence;

    #[test]
    fn replay_sequence_must_be_strictly_increasing_after_last_seen() {
        assert!(validate_sequence(&[json!({"serverSeq": 4}), json!({"serverSeq": 5})], 3).is_ok());
        assert!(validate_sequence(&[json!({"serverSeq": 4}), json!({"serverSeq": 4})], 3).is_err());
        assert!(validate_sequence(&[json!({"serverSeq": 3})], 3).is_err());
    }
}
