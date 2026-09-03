//! Validation of a complete turn lifecycle in a reconnect replay.

use std::collections::HashMap;

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
    let mut response_parts = HashMap::new();
    let mut saw_response_activity = false;
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
                if !saw_started {
                    return Err(ConformanceError::Options(format!(
                        "chat/responsePart precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
                let kind = envelope
                    .pointer("/action/part/kind")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConformanceError::Options(
                            "replayed chat/responsePart omitted its kind".to_owned(),
                        )
                    })?;
                let part_id = envelope
                    .pointer("/action/part/id")
                    .or_else(|| envelope.pointer("/action/part/toolCall/toolCallId"))
                    .and_then(Value::as_str);
                if matches!(kind, "markdown" | "reasoning") && part_id.is_none() {
                    return Err(ConformanceError::Options(format!(
                        "replayed {kind} response part omitted its part id"
                    )));
                }
                if let Some(part_id) = part_id.filter(|id| !id.is_empty()) {
                    if response_parts
                        .insert(part_id.to_owned(), kind.to_owned())
                        .is_some()
                    {
                        return Err(ConformanceError::Options(format!(
                            "replayed chat/responsePart duplicated part id {part_id:?}"
                        )));
                    }
                }
                if kind == "markdown" {
                    if let Some(initial) = envelope
                        .pointer("/action/part/content")
                        .and_then(Value::as_str)
                    {
                        content.push_str(initial);
                    }
                }
                saw_response_activity = true;
            }
            "chat/delta" => {
                let part_id = envelope
                    .pointer("/action/partId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        ConformanceError::Options(
                            "replayed chat/delta omitted its part id".to_owned(),
                        )
                    })?;
                if response_parts.get(part_id).map(String::as_str) != Some("markdown") {
                    return Err(ConformanceError::Options(
                        "replayed chat/delta did not target an opened markdown part".to_owned(),
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
                saw_response_activity = true;
            }
            "chat/usage" => {
                if !saw_started {
                    return Err(ConformanceError::Options(format!(
                        "chat/usage precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
            }
            "chat/turnComplete" | "chat/error" => {
                if !saw_started {
                    return Err(ConformanceError::Options(format!(
                        "{action_type} precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
                saw_response_activity |= action_type == "chat/error";
                terminal = Some(action_type);
            }
            _ => {
                if !saw_started {
                    return Err(ConformanceError::Options(format!(
                        "{action_type} precedes chat/turnStarted for turn {turn_id:?}"
                    )));
                }
                saw_response_activity = true;
            }
        }
    }

    if !saw_started {
        return Err(ConformanceError::Options(format!(
            "replay omitted chat/turnStarted for turn {turn_id:?}"
        )));
    }
    let terminal = terminal.ok_or_else(|| {
        ConformanceError::Options(format!(
            "replay omitted a terminal action for turn {turn_id:?}"
        ))
    })?;
    if terminal == "chat/turnComplete" && !saw_response_activity {
        return Err(ConformanceError::Options(
            "a completed replayed turn contained no response activity".to_owned(),
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

pub(super) fn validate_sequence(actions: &[Value], last_seen: i64) -> Result<(), ConformanceError> {
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

    use super::{validate_replayed_turn, validate_sequence};

    #[test]
    fn replay_sequence_must_be_strictly_increasing_after_last_seen() {
        assert!(validate_sequence(&[json!({"serverSeq": 4}), json!({"serverSeq": 5})], 3).is_ok());
        assert!(validate_sequence(&[json!({"serverSeq": 4}), json!({"serverSeq": 4})], 3).is_err());
        assert!(validate_sequence(&[json!({"serverSeq": 3})], 3).is_err());
    }

    #[test]
    fn replay_accepts_reasoning_before_markdown() {
        let channel = "ahp-chat:/test";
        let turn_id = "turn";
        let actions = vec![
            json!({
                "channel": channel,
                "serverSeq": 2,
                "action": {
                    "type": "chat/turnStarted",
                    "turnId": turn_id,
                    "startedAt": "2026-01-02T03:04:05.000Z",
                    "message": { "text": "hello", "origin": { "kind": "user" } }
                }
            }),
            json!({
                "channel": channel,
                "serverSeq": 3,
                "action": {
                    "type": "chat/responsePart",
                    "turnId": turn_id,
                    "part": { "kind": "reasoning", "id": "reasoning", "content": "thinking" }
                }
            }),
            json!({
                "channel": channel,
                "serverSeq": 4,
                "action": {
                    "type": "chat/responsePart",
                    "turnId": turn_id,
                    "part": { "kind": "markdown", "id": "answer", "content": "hello " }
                }
            }),
            json!({
                "channel": channel,
                "serverSeq": 5,
                "action": {
                    "type": "chat/delta",
                    "turnId": turn_id,
                    "partId": "answer",
                    "content": "world"
                }
            }),
            json!({
                "channel": channel,
                "serverSeq": 6,
                "action": {
                    "type": "chat/turnComplete",
                    "turnId": turn_id,
                    "duration": 1
                }
            }),
        ];

        validate_replayed_turn(&actions, 1, channel, turn_id, Some("hello world"))
            .expect("reasoning and markdown parts should form a valid replay lifecycle");
    }
}
