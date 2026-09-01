//! Local shell approval and persisted-command conformance cases.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use serde_json::json;

use super::super::{ConformanceError, Probe};
use super::approval::{assert_confirmation, await_confirmation, confirm, is_setting};
use super::helpers::{await_tool_complete, await_tool_ready, start_fixture_tool_turn, ToolReady};
use super::lifecycle::complete_fixture_turn;
use super::workspace::{file_uri, workspace_chat};

const SHELL_OUTPUT: &str = "AHP_SHELL_OUTPUT";
const PERSISTENT_COMMAND_MARKER: &str = "shell-persistent-command.txt";
const DIFFERENT_COMMAND_MARKER: &str = "shell-persistent-different.txt";

pub(in super::super) async fn shell_approval(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<String, ConformanceError> {
    let denied_marker = workspace.join("shell-denied.txt");
    let allowed_marker = workspace.join("shell-allowed.txt");
    remove_if_exists(&denied_marker)?;
    remove_if_exists(&allowed_marker)?;
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, "shell-approval", &file_uri(workspace)?).await?;

    let deny_turn = "shell-denied";
    let denied = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        deny_turn,
        "run_shell",
        json!({ "command": shell_command("shell-denied.txt") }),
        "shell denial completed",
    )
    .await?;
    let ready = await_tool_ready(&mut probe, &session.chat, deny_turn, &denied).await?;
    assert_pending_approval(&ready)?;
    assert_marker_absent(&denied_marker)?;
    confirm(&mut probe, &session.chat, deny_turn, &denied, "deny").await?;
    let confirmation = await_confirmation(&mut probe, &session.chat, deny_turn, &denied).await?;
    assert_confirmation(&confirmation, false, "deny", false)?;
    let result = await_tool_complete(&mut probe, &session.chat, deny_turn, &denied).await?;
    if result.success || result.error.is_none() || result.text.is_empty() {
        return Err(ConformanceError::Options(
            "denied shell command did not complete with a truthful failed result".to_owned(),
        ));
    }
    complete_fixture_turn(
        &mut probe,
        &session.chat,
        deny_turn,
        "shell denial completed",
    )
    .await?;
    assert_marker_absent(&denied_marker)?;

    let allow_turn = "shell-allowed";
    let allowed = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        allow_turn,
        "run_shell",
        json!({ "command": shell_command("shell-allowed.txt") }),
        "shell allow completed",
    )
    .await?;
    let ready = await_tool_ready(&mut probe, &session.chat, allow_turn, &allowed).await?;
    assert_pending_approval(&ready)?;
    assert_marker_absent(&allowed_marker)?;
    confirm(
        &mut probe,
        &session.chat,
        allow_turn,
        &allowed,
        "allow-once",
    )
    .await?;
    let confirmation = await_confirmation(&mut probe, &session.chat, allow_turn, &allowed).await?;
    assert_confirmation(&confirmation, true, "allow-once", true)?;
    let result = await_tool_complete(&mut probe, &session.chat, allow_turn, &allowed).await?;
    if !result.success || !result.text.contains(SHELL_OUTPUT) {
        return Err(ConformanceError::Options(
            "allowed shell command did not report its deterministic output".to_owned(),
        ));
    }
    complete_fixture_turn(
        &mut probe,
        &session.chat,
        allow_turn,
        "shell allow completed",
    )
    .await?;
    assert_marker(&allowed_marker)?;
    probe.close().await?;
    Ok("requires advertised deny/allow approval and executes only after allow-once".to_owned())
}

pub(in super::super) async fn approve_always(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<(), ConformanceError> {
    let marker = workspace.join(PERSISTENT_COMMAND_MARKER);
    remove_if_exists(&marker)?;
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, "shell-persist-first", &file_uri(workspace)?).await?;
    let turn = "shell-persistent-first";
    let call = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        turn,
        "run_shell",
        json!({ "command": shell_command(PERSISTENT_COMMAND_MARKER) }),
        "persistent approval saved",
    )
    .await?;
    assert_pending_approval(&await_tool_ready(&mut probe, &session.chat, turn, &call).await?)?;
    assert_marker_absent(&marker)?;
    confirm(
        &mut probe,
        &session.chat,
        turn,
        &call,
        "always-allow-command",
    )
    .await?;
    let confirmation = await_confirmation(&mut probe, &session.chat, turn, &call).await?;
    assert_confirmation(&confirmation, true, "always-allow-command", true)?;
    if !is_setting(confirmation.confirmed.as_deref()) {
        return Err(ConformanceError::Options(
            "always-allow confirmation was not authoritative setting approval".to_owned(),
        ));
    }
    let result = await_tool_complete(&mut probe, &session.chat, turn, &call).await?;
    if !result.success || !result.text.contains(SHELL_OUTPUT) {
        return Err(ConformanceError::Options(
            "always-allowed command did not complete successfully".to_owned(),
        ));
    }
    complete_fixture_turn(&mut probe, &session.chat, turn, "persistent approval saved").await?;
    assert_marker(&marker)?;
    probe.close().await
}

pub(in super::super) async fn verify_persisted_approval(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<(), ConformanceError> {
    let marker = workspace.join(PERSISTENT_COMMAND_MARKER);
    remove_if_exists(&marker)?;
    let mut probe = Probe::connect(target, timeout).await?;
    let session =
        workspace_chat(&mut probe, "shell-persist-restarted", &file_uri(workspace)?).await?;
    let turn = "shell-persistent-restarted";
    let call = start_fixture_tool_turn(
        &mut probe,
        &session.chat,
        turn,
        "run_shell",
        json!({ "command": shell_command(PERSISTENT_COMMAND_MARKER) }),
        "persisted approval used",
    )
    .await?;
    let ready = await_tool_ready(&mut probe, &session.chat, turn, &call).await?;
    if !is_setting(ready.confirmed.as_deref()) || !ready.option_ids.is_empty() {
        return Err(ConformanceError::Options(
            "persisted shell approval did not auto-confirm with setting".to_owned(),
        ));
    }
    let result = await_tool_complete(&mut probe, &session.chat, turn, &call).await?;
    if !result.success || !result.text.contains(SHELL_OUTPUT) {
        return Err(ConformanceError::Options(
            "persisted shell approval did not run to successful output".to_owned(),
        ));
    }
    complete_fixture_turn(&mut probe, &session.chat, turn, "persisted approval used").await?;
    assert_marker(&marker)?;
    verify_different_command_requires_approval(&mut probe, &session.chat, workspace).await?;
    probe.close().await
}

pub(super) fn shell_command(marker: &str) -> String {
    #[cfg(windows)]
    {
        format!("echo {SHELL_OUTPUT} > {marker} & echo {SHELL_OUTPUT}")
    }
    #[cfg(not(windows))]
    {
        format!("printf %s {SHELL_OUTPUT} > {marker}; printf %s {SHELL_OUTPUT}")
    }
}

fn assert_pending_approval(ready: &ToolReady) -> Result<(), ConformanceError> {
    let expected = BTreeSet::from([
        "allow-once".to_owned(),
        "always-allow-command".to_owned(),
        "deny".to_owned(),
    ]);
    if ready.confirmed.is_none()
        && ready.option_ids.iter().cloned().collect::<BTreeSet<_>>() == expected
    {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "pending shell approval did not expose the required option IDs".to_owned(),
        ))
    }
}

async fn verify_different_command_requires_approval(
    probe: &mut Probe,
    chat: &str,
    workspace: &Path,
) -> Result<(), ConformanceError> {
    let marker = workspace.join(DIFFERENT_COMMAND_MARKER);
    remove_if_exists(&marker)?;
    let turn = "shell-persistent-different";
    let call = start_fixture_tool_turn(
        probe,
        chat,
        turn,
        "run_shell",
        json!({ "command": shell_command(DIFFERENT_COMMAND_MARKER) }),
        "different command denied",
    )
    .await?;
    let ready = await_tool_ready(probe, chat, turn, &call).await?;
    assert_pending_approval(&ready)?;
    assert_marker_absent(&marker)?;
    confirm(probe, chat, turn, &call, "deny").await?;
    let confirmation = await_confirmation(probe, chat, turn, &call).await?;
    assert_confirmation(&confirmation, false, "deny", false)?;
    let result = await_tool_complete(probe, chat, turn, &call).await?;
    if result.success || result.error.is_none() {
        return Err(ConformanceError::Options(
            "different command did not receive a truthful denied result".to_owned(),
        ));
    }
    complete_fixture_turn(probe, chat, turn, "different command denied").await?;
    assert_marker_absent(&marker)
}

fn assert_marker(marker: &Path) -> Result<(), ConformanceError> {
    if std::fs::read_to_string(marker)?.trim() == SHELL_OUTPUT {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "shell marker {marker:?} did not contain the expected output"
        )))
    }
}

fn assert_marker_absent(marker: &Path) -> Result<(), ConformanceError> {
    if marker.exists() {
        Err(ConformanceError::Options(format!(
            "shell command executed before approval and created {marker:?}"
        )))
    } else {
        Ok(())
    }
}

fn remove_if_exists(path: &Path) -> Result<(), ConformanceError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ConformanceError::Io(error)),
    }
}
