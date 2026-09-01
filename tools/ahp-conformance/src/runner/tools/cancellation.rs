//! Local cancellation coverage for an approved delayed shell command.

use std::path::Path;
use std::time::Duration;

use serde_json::json;

use super::super::turns::assert_cancelled_snapshot;
use super::super::{ConformanceError, Probe};
use super::approval::{await_confirmation, confirm};
use super::helpers::{await_tool_ready, start_fixture_tool_turn};
use super::workspace::{file_uri, workspace_chat};

const CANCELLATION_WAIT: Duration = Duration::from_secs(3);

pub(in super::super) async fn tool_cancellation(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<String, ConformanceError> {
    let marker = workspace.join("shell-cancelled.txt");
    let _ = std::fs::remove_file(&marker);
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, "tool-cancellation", &file_uri(workspace)?).await?;
    let turn = "tool-cancellation";
    let call = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        turn,
        "run_shell",
        json!({ "command": delayed_shell_command("shell-cancelled.txt") }),
        "this response must not arrive",
    )
    .await?;
    let ready = await_tool_ready(&mut probe, &session.chat, turn, &call).await?;
    if ready.confirmed.is_some() {
        return Err(ConformanceError::Options(
            "delayed shell command bypassed user approval".to_owned(),
        ));
    }
    confirm(&mut probe, &session.chat, turn, &call, "allow-once").await?;
    let confirmation = await_confirmation(&mut probe, &session.chat, turn, &call).await?;
    if !confirmation.approved || confirmation.selected_option_id != "allow-once" {
        return Err(ConformanceError::Options(
            "delayed shell command was not authoritatively approved".to_owned(),
        ));
    }
    probe
        .dispatch_action(
            &session.chat,
            json!({ "type": "chat/turnCancelled", "turnId": turn }),
        )
        .await?;
    let cancelled = probe
        .wait_action(&session.chat, "chat/turnCancelled")
        .await?;
    if cancelled
        .pointer("/params/action/turnId")
        .and_then(serde_json::Value::as_str)
        != Some(turn)
    {
        return Err(ConformanceError::Options(
            "cancellation action targeted the wrong turn".to_owned(),
        ));
    }
    let cancelled_sequence = Probe::action_server_sequence(&cancelled)?;
    probe
        .assert_no_post_cancel_activity_for_turn(
            &session.chat,
            turn,
            cancelled_sequence,
            CANCELLATION_WAIT,
        )
        .await?;
    let snapshot = probe
        .request("subscribe", json!({ "channel": &session.chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| {
            ConformanceError::Options("cancelled tool chat omitted a snapshot".to_owned())
        })?;
    assert_cancelled_snapshot(&snapshot, turn)?;
    if marker.exists() {
        return Err(ConformanceError::Options(
            "cancelled delayed shell command created its later marker".to_owned(),
        ));
    }
    cancel_pending_confirmation(&mut probe, &session.chat, workspace).await?;
    probe.close().await?;
    Ok(
        "cancels approved and pending shell calls without later activity, side effects, or live approvals"
            .to_owned(),
    )
}

async fn cancel_pending_confirmation(
    probe: &mut Probe,
    chat: &str,
    workspace: &Path,
) -> Result<(), ConformanceError> {
    let marker = workspace.join("shell-pending-cancelled.txt");
    let _ = std::fs::remove_file(&marker);
    let turn = "tool-cancellation-pending";
    let call = start_fixture_tool_turn(
        probe,
        chat,
        turn,
        "run_shell",
        json!({ "command": super::shell::shell_command("shell-pending-cancelled.txt") }),
        "this response must not arrive",
    )
    .await?;
    let ready = await_tool_ready(probe, chat, turn, &call).await?;
    if ready.confirmed.is_some() || ready.option_ids.is_empty() {
        return Err(ConformanceError::Options(
            "pending cancellation test did not receive shell confirmation".to_owned(),
        ));
    }
    if marker.exists() {
        return Err(ConformanceError::Options(
            "pending shell command executed before cancellation".to_owned(),
        ));
    }
    probe
        .dispatch_action(
            chat,
            json!({ "type": "chat/turnCancelled", "turnId": turn }),
        )
        .await?;
    let cancelled = probe.wait_action(chat, "chat/turnCancelled").await?;
    let cancelled_sequence = Probe::action_server_sequence(&cancelled)?;
    probe
        .assert_no_post_cancel_activity_for_turn(
            chat,
            turn,
            cancelled_sequence,
            Duration::from_secs(1),
        )
        .await?;
    let snapshot = probe
        .request("subscribe", json!({ "channel": chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| {
            ConformanceError::Options("pending cancellation snapshot was omitted".to_owned())
        })?;
    assert_cancelled_snapshot(&snapshot, turn)?;
    if marker.exists() {
        return Err(ConformanceError::Options(
            "pending cancelled shell command created a marker".to_owned(),
        ));
    }
    Ok(())
}

fn delayed_shell_command(marker: &str) -> String {
    #[cfg(windows)]
    {
        format!("ping 127.0.0.1 -n 3 > NUL & echo cancelled > {marker}")
    }
    #[cfg(not(windows))]
    {
        format!("sleep 2; printf %s cancelled > {marker}")
    }
}
