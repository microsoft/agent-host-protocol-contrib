//! Observable turn streaming and cancellation probes.
//!
//! Internal conformance-runner module.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{json, Value};

use super::advanced::{prompt_for_model, ready_chat, start_turn};
use super::{ConformanceError, Probe, ScenarioCheck, ScenarioRun};

const POST_CANCEL_QUIESCENCE: Duration = Duration::from_secs(1);

pub(super) async fn streaming_lifecycle(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
    auth_token: Option<&str>,
) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    probe.set_auth_token(auth_token);
    let (session, chat, _) = match ready_chat(&mut probe, "stream").await {
        Ok(ready) => ready,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = probe.request("subscribe", json!({ "channel": chat })).await {
        return run.abort(error);
    }
    let expected = "streamed output from the deterministic model";
    if let Err(error) = start_turn(
        &mut probe,
        &chat,
        "stream-turn",
        prompt_for_model(exact_output, expected, 4, 0),
    )
    .await
    {
        return run.abort(error);
    }
    let started = probe
        .wait_action(&chat, "chat/turnStarted")
        .await
        .map_err(|error| {
            ConformanceError::Options(format!("waiting for chat/turnStarted: {error}"))
        });
    let started = match started {
        Ok(started) => started,
        Err(error) => return run.abort(error),
    };
    match require_turn_id(&started, "stream-turn") {
        Ok(()) => run.push(ScenarioCheck::passed(
            "turn-started",
            "turnStarted identified the dispatched turn",
        )),
        Err(error) => run.push(ScenarioCheck::failed("turn-started", error)),
    }
    let response_part =
        match wait_for_markdown_response_part(&mut probe, &chat, "stream-turn").await {
            Ok(response_part) => response_part,
            Err(error) => return run.abort(error),
        };
    let response_part_check = require_turn_id(&response_part, "stream-turn").and_then(|()| {
        if response_part
            .pointer("/params/action/part/kind")
            .and_then(Value::as_str)
            == Some("markdown")
        {
            Ok(())
        } else {
            Err(ConformanceError::Options(
                "chat/responsePart was not a markdown part".to_owned(),
            ))
        }
    });
    match response_part_check {
        Ok(()) => run.push(ScenarioCheck::passed(
            "response-part-markdown",
            "turn opened a markdown response part",
        )),
        Err(error) => run.push(ScenarioCheck::failed("response-part-markdown", error)),
    }
    let mut opened_markdown_parts = HashMap::new();
    let mut ordering_error = record_markdown_part(&response_part, &mut opened_markdown_parts).err();
    let mut response = String::new();
    loop {
        let action = match probe
            .wait_action_any(
                &chat,
                &[
                    "chat/responsePart",
                    "chat/delta",
                    "chat/turnComplete",
                    "chat/error",
                ],
            )
            .await
        {
            Ok(action) => action,
            Err(error) => return run.abort(error),
        };
        match action
            .pointer("/params/action/type")
            .and_then(Value::as_str)
        {
            Some("chat/responsePart") => {
                if let Err(error) = require_turn_id(&action, "stream-turn") {
                    return run.abort(error);
                }
                if is_markdown_response_part(&action) {
                    if let Err(error) = record_markdown_part(&action, &mut opened_markdown_parts) {
                        if ordering_error.is_none() {
                            ordering_error = Some(error);
                        }
                    }
                }
            }
            Some("chat/delta") => {
                if let Err(error) = require_turn_id(&action, "stream-turn") {
                    return run.abort(error);
                }
                if let Err(error) = validate_delta_target(&action, &opened_markdown_parts) {
                    if ordering_error.is_none() {
                        ordering_error = Some(error);
                    }
                }
                let content = match action
                    .pointer("/params/action/content")
                    .and_then(Value::as_str)
                {
                    Some(content) => content,
                    None => return run.abort("delta omitted content"),
                };
                response.push_str(content);
            }
            Some("chat/turnComplete") => {
                if let Err(error) = require_turn_id(&action, "stream-turn") {
                    return run.abort(error);
                }
                run.push(ScenarioCheck::passed(
                    "turn-complete",
                    "turn emitted a matching terminal completion",
                ));
                break;
            }
            Some("chat/error") => {
                return run.abort(
                    terminal_error_detail(&action)
                        .unwrap_or_else(|| "turn ended in chat/error".to_owned()),
                );
            }
            _ => return run.abort("unexpected action selection"),
        }
    }
    match ordering_error {
        Some(error) => run.push(ScenarioCheck::failed("response-part-before-delta", error)),
        None => run.push(ScenarioCheck::passed(
            "response-part-before-delta",
            "every markdown delta followed creation of its target response part",
        )),
    }
    if response.is_empty() {
        run.push(ScenarioCheck::failed(
            "response-deltas",
            "streamed turn did not produce text",
        ));
    } else if exact_output && response != expected {
        run.push(ScenarioCheck::failed(
            "response-deltas",
            ConformanceError::Options(format!(
                "deterministic response was {response:?}, expected {expected:?}"
            )),
        ));
    } else {
        run.push(ScenarioCheck::passed(
            "response-deltas",
            "ordered deltas reconstructed non-empty response text",
        ));
    }
    let summary = match probe
        .wait_notification_for_resource("root/sessionSummaryChanged", "ahp-root://", &session)
        .await
    {
        Ok(summary) => summary,
        Err(error) => return run.abort(error),
    };
    run.push(ScenarioCheck::passed(
        "root-session-summary-changed",
        "turn activity broadcast root/sessionSummaryChanged",
    ));
    let changes = summary
        .pointer("/params/changes")
        .and_then(Value::as_object);
    if changes.is_some_and(|changes| {
        !changes.contains_key("resource")
            && !changes.contains_key("provider")
            && !changes.contains_key("createdAt")
    }) {
        run.push(ScenarioCheck::passed(
            "session-summary-identity-omitted",
            "session summary changes omitted immutable identity fields",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "session-summary-identity-omitted",
            "session summary changes included immutable identity fields or omitted changes",
        ));
    }
    run
}

pub(super) async fn cancellation(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
    auth_token: Option<&str>,
) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    probe.set_auth_token(auth_token);
    let (_session, chat, _) = match ready_chat(&mut probe, "cancel").await {
        Ok(ready) => ready,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = probe.request("subscribe", json!({ "channel": chat })).await {
        return run.abort(error);
    }
    if let Err(error) = start_turn(
        &mut probe,
        &chat,
        "cancel-turn",
        prompt_for_model(exact_output, "this must be cancelled", 2, 750),
    )
    .await
    {
        return run.abort(error);
    }
    let response_part = match wait_for_response_part(&mut probe, &chat, "cancel-turn").await {
        Ok(response_part) => response_part,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = require_turn_id(&response_part, "cancel-turn") {
        return run.abort(error);
    }
    if let Err(error) = probe
        .dispatch_action(
            &chat,
            json!({
                "type": "chat/turnCancelled",
                "turnId": "cancel-turn",
            }),
        )
        .await
    {
        return run.abort(error);
    }
    let cancelled_action = match probe.wait_action(&chat, "chat/turnCancelled").await {
        Ok(cancelled) => cancelled,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = require_turn_id(&cancelled_action, "cancel-turn") {
        run.push(ScenarioCheck::failed("cancellation-terminal", error));
        return run;
    }
    run.push(ScenarioCheck::passed(
        "cancellation-terminal",
        "host emitted the matching terminal cancellation",
    ));
    let cancelled_sequence = match Probe::action_server_sequence(&cancelled_action) {
        Ok(sequence) => sequence,
        Err(error) => return run.abort(error),
    };
    match probe
        .assert_no_post_cancel_activity_for_turn(
            &chat,
            "cancel-turn",
            cancelled_sequence,
            POST_CANCEL_QUIESCENCE,
        )
        .await
    {
        Ok(()) => run.push(ScenarioCheck::passed(
            "cancellation-quiescence",
            "no activity followed the terminal cancellation",
        )),
        Err(error) => run.push(ScenarioCheck::failed("cancellation-quiescence", error)),
    }
    let snapshot = match probe.request("subscribe", json!({ "channel": chat })).await {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let snapshot = match snapshot.pointer("/result/snapshot/state").cloned() {
        Some(snapshot) => snapshot,
        None => {
            run.push(ScenarioCheck::failed(
                "cancellation-snapshot",
                "cancelled chat subscription omitted a snapshot",
            ));
            return run;
        }
    };
    match assert_cancelled_snapshot(&snapshot, "cancel-turn") {
        Ok(()) => run.push(ScenarioCheck::passed(
            "cancellation-snapshot",
            "snapshot retained the cancelled terminal turn",
        )),
        Err(error) => run.push(ScenarioCheck::failed("cancellation-snapshot", error)),
    }
    run
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

async fn wait_for_response_part(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
) -> Result<Value, ConformanceError> {
    let action = probe
        .wait_action_any(chat, &["chat/responsePart", "chat/error"])
        .await
        .map_err(|error| {
            ConformanceError::Options(format!(
                "waiting for chat/responsePart or chat/error: {error}"
            ))
        })?;
    require_turn_id(&action, turn_id)?;
    if let Some(detail) = terminal_error_detail(&action) {
        return Err(ConformanceError::Options(detail));
    }
    Ok(action)
}

async fn wait_for_markdown_response_part(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
) -> Result<Value, ConformanceError> {
    loop {
        let action = probe
            .wait_action_any(
                chat,
                &["chat/responsePart", "chat/error", "chat/turnComplete"],
            )
            .await
            .map_err(|error| {
                ConformanceError::Options(format!(
                    "waiting for a markdown chat/responsePart: {error}"
                ))
            })?;
        require_turn_id(&action, turn_id)?;
        if let Some(detail) = terminal_error_detail(&action) {
            return Err(ConformanceError::Options(detail));
        }
        match action
            .pointer("/params/action/type")
            .and_then(Value::as_str)
        {
            Some("chat/responsePart") if is_markdown_response_part(&action) => return Ok(action),
            Some("chat/responsePart") => {}
            Some("chat/turnComplete") => {
                return Err(ConformanceError::Options(
                    "turn completed without a markdown response part".to_owned(),
                ));
            }
            _ => {
                return Err(ConformanceError::Options(
                    "unexpected action while waiting for a markdown response part".to_owned(),
                ));
            }
        }
    }
}

fn is_markdown_response_part(action: &Value) -> bool {
    action
        .pointer("/params/action/part/kind")
        .and_then(Value::as_str)
        == Some("markdown")
}

fn record_markdown_part(
    action: &Value,
    opened: &mut HashMap<String, i64>,
) -> Result<(), ConformanceError> {
    let part_id = action
        .pointer("/params/action/part/id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ConformanceError::Options("markdown response part omitted its id".to_owned())
        })?;
    opened.insert(part_id.to_owned(), Probe::action_server_sequence(action)?);
    Ok(())
}

fn validate_delta_target(
    action: &Value,
    opened: &HashMap<String, i64>,
) -> Result<(), ConformanceError> {
    let part_id = action
        .pointer("/params/action/partId")
        .and_then(Value::as_str)
        .ok_or_else(|| ConformanceError::Options("chat/delta omitted partId".to_owned()))?;
    let created_at = opened.get(part_id).ok_or_else(|| {
        ConformanceError::Options(format!(
            "chat/delta targeted response part {part_id:?} before it was created"
        ))
    })?;
    let delta_sequence = Probe::action_server_sequence(action)?;
    if delta_sequence <= *created_at {
        return Err(ConformanceError::Options(format!(
            "chat/delta at serverSeq {delta_sequence} preceded response part {part_id:?} at serverSeq {created_at}"
        )));
    }
    Ok(())
}

fn terminal_error_detail(action: &Value) -> Option<String> {
    (action
        .pointer("/params/action/type")
        .and_then(Value::as_str)
        == Some("chat/error"))
    .then(|| {
        let error_type = action
            .pointer("/params/action/part/error/errorType")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let message = action
            .pointer("/params/action/part/error/message")
            .and_then(Value::as_str)
            .unwrap_or("host omitted the error message");
        format!("turn ended in chat/error ({error_type}): {message}")
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{is_markdown_response_part, terminal_error_detail};

    #[test]
    fn terminal_error_detail_surfaces_type_and_message() {
        let action = json!({
            "jsonrpc": "2.0",
            "method": "action",
            "params": {
                "channel": "ahp-chat:/test",
                "serverSeq": 1,
                "action": {
                    "type": "chat/error",
                    "turnId": "turn",
                    "duration": 1,
                    "part": {
                        "kind": "error",
                        "error": {
                            "errorType": "provider",
                            "message": "model unavailable"
                        }
                    }
                }
            }
        });

        assert_eq!(
            terminal_error_detail(&action).as_deref(),
            Some("turn ended in chat/error (provider): model unavailable")
        );
    }

    #[test]
    fn markdown_detection_allows_preceding_reasoning_parts() {
        let reasoning = json!({
            "params": { "action": { "type": "chat/responsePart", "part": { "kind": "reasoning" } } }
        });
        let markdown = json!({
            "params": { "action": { "type": "chat/responsePart", "part": { "kind": "markdown" } } }
        });

        assert!(!is_markdown_response_part(&reasoning));
        assert!(is_markdown_response_part(&markdown));
    }
}
