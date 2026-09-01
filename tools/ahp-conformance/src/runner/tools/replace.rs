//! Local replace-string lifecycle, confinement, and recovery conformance case.

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::super::{ConformanceError, Probe};
use super::helpers::{
    await_tool_complete, await_tool_ready, parse_tool_result, start_fixture_tool_turn,
};
use super::lifecycle::complete_fixture_turn;
use super::replay::assert_replay_tool_result;
use super::workspace::{file_uri, workspace_chat};

pub(in super::super) async fn replace_string_tool(
    target: &str,
    timeout: Duration,
    workspace: &Path,
    outside_workspace: &Path,
) -> Result<String, ConformanceError> {
    let note = workspace.join("replace-note.txt");
    let outside = outside_workspace.join("outside-note.txt");
    std::fs::write(&note, "before OLD after")?;
    std::fs::write(&outside, "outside remains untouched")?;

    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, "replace-string", &file_uri(workspace)?).await?;
    let turn_id = "replace-string-success";
    let call = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        turn_id,
        "replace_string",
        json!({ "path": "replace-note.txt", "old_string": "OLD", "new_string": "NEW" }),
        "replace completed",
    )
    .await?;
    let ready = await_tool_ready(&mut probe, &session.chat, turn_id, &call).await?;
    if ready.confirmed.as_deref() != Some("not-needed") || !ready.option_ids.is_empty() {
        return Err(ConformanceError::Options(
            "replace_string was not automatically confirmed as not-needed".to_owned(),
        ));
    }
    let complete = await_tool_complete(&mut probe, &session.chat, turn_id, &call).await?;
    if !complete.success || complete.text.is_empty() {
        return Err(ConformanceError::Options(
            "replace_string did not report a successful result".to_owned(),
        ));
    }
    complete_fixture_turn(&mut probe, &session.chat, turn_id, "replace completed").await?;
    if std::fs::read_to_string(&note)? != "before NEW after" {
        return Err(ConformanceError::Options(
            "replace_string did not change the workspace file exactly once".to_owned(),
        ));
    }

    let snapshot = probe
        .request("subscribe", json!({ "channel": &session.chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("replace chat omitted a snapshot".to_owned()))?;
    assert_completed_snapshot_tool(&snapshot, turn_id, &call.id, true)?;
    probe.close().await?;

    let mut reconnected = Probe::connect(target, timeout).await?;
    let replay = reconnected
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": "replace-string-reconnect",
                "lastSeenServerSeq": 0,
                "subscriptions": ["ahp-root://", &session.chat],
            }),
        )
        .await?;
    if replay.pointer("/result/type").and_then(Value::as_str) != Some("replay") {
        return Err(ConformanceError::Options(
            "replace reconnect did not return replay actions".to_owned(),
        ));
    }
    assert_replay_tool_result(&replay, &session.chat, turn_id, &call, true)?;
    reconnected.close().await?;

    let mut negative_probe = Probe::connect(target, timeout).await?;
    let negative = workspace_chat(
        &mut negative_probe,
        "replace-string-negative",
        &file_uri(workspace)?,
    )
    .await?;
    let negative_turn = "replace-string-escape";
    let negative_call = start_fixture_tool_turn(
        &mut negative_probe,
        &negative.chat,
        negative_turn,
        "replace_string",
        json!({
            "path": "../outside/outside-note.txt",
            "old_string": "outside",
            "new_string": "changed",
        }),
        "replace failure reported",
    )
    .await?;
    let ready = await_tool_ready(
        &mut negative_probe,
        &negative.chat,
        negative_turn,
        &negative_call,
    )
    .await?;
    if ready.confirmed.as_deref() != Some("not-needed") {
        return Err(ConformanceError::Options(
            "failed replace_string was not automatically confirmed".to_owned(),
        ));
    }
    let failure = await_tool_complete(
        &mut negative_probe,
        &negative.chat,
        negative_turn,
        &negative_call,
    )
    .await?;
    if failure.success || failure.error.is_none() || failure.text.is_empty() {
        return Err(ConformanceError::Options(
            "workspace escape did not return a truthful failed tool result".to_owned(),
        ));
    }
    complete_fixture_turn(
        &mut negative_probe,
        &negative.chat,
        negative_turn,
        "replace failure reported",
    )
    .await?;
    if std::fs::read_to_string(&note)? != "before NEW after"
        || std::fs::read_to_string(&outside)? != "outside remains untouched"
    {
        return Err(ConformanceError::Options(
            "failed replace_string changed a workspace or outside file".to_owned(),
        ));
    }
    negative_probe.close().await?;
    Ok(
        "replaces one workspace string, persists its result, and safely reports an escape"
            .to_owned(),
    )
}

fn assert_completed_snapshot_tool(
    snapshot: &Value,
    turn_id: &str,
    tool_call_id: &str,
    expected_success: bool,
) -> Result<(), ConformanceError> {
    let turn = snapshot
        .get("turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns
                .iter()
                .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        })
        .ok_or_else(|| {
            ConformanceError::Options("completed replace turn is not in snapshot".to_owned())
        })?;
    let tool_call = turn
        .get("responseParts")
        .and_then(Value::as_array)
        .and_then(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("toolCall"))
                .find(|tool| tool.get("toolCallId").and_then(Value::as_str) == Some(tool_call_id))
        })
        .ok_or_else(|| {
            ConformanceError::Options("snapshot omitted tool response part".to_owned())
        })?;
    if tool_call.get("status").and_then(Value::as_str) != Some("completed") {
        return Err(ConformanceError::Options(
            "snapshot tool response part is not completed".to_owned(),
        ));
    }
    let result = parse_tool_result(tool_call)?;
    if result.success != expected_success || result.text.is_empty() {
        return Err(ConformanceError::Options(
            "snapshot did not retain completed tool result content".to_owned(),
        ));
    }
    Ok(())
}
