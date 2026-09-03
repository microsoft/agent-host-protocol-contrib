//! Bounded platform-shell execution and exact command approval identity.
#[cfg(not(any(unix, windows)))]
compile_error!("workspace-agent-tools supports Unix and Windows targets only");

use std::fmt;
use std::process::Stdio;
use std::time::Duration;

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use serde::Deserialize;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::output::{
    abort_capture, captured_output, finish_capture, new_capture, start_capture, Capture,
};
use crate::ToolExecution;
/// Default timeout for `run_shell`.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 30;
/// Longest timeout accepted by `run_shell`.
pub const MAX_TIMEOUT_SECONDS: u64 = 300;
/// Maximum combined stdout and stderr bytes retained in a result.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// An approval identity for one exact, trimmed shell command.
/// This deliberately does not normalize executable names, command arguments, case,
/// or shell syntax. It must be stored and compared by exact equality by the host.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ExactCommandApproval(String);

impl ExactCommandApproval {
    /// Creates an approval key from a non-empty command after trimming its ends.
    pub fn new(command: impl AsRef<str>) -> Result<Self, String> {
        let command = command.as_ref().trim();
        if command.is_empty() {
            return Err("command must not be empty".into());
        }
        Ok(Self(command.into()))
    }

    /// Returns the exact trimmed command which is the approval identity.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Compares this key only to a candidate's exact trimmed command text.
    #[must_use]
    pub fn matches_command(&self, command: &str) -> bool {
        self.0 == command.trim()
    }
}

impl AsRef<str> for ExactCommandApproval {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for ExactCommandApproval {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A validated shell command which has not been run.
#[derive(Clone, Debug)]
pub struct PreparedShell {
    approval_key: ExactCommandApproval,
    timeout: Duration,
    display_executable: String,
}

impl PreparedShell {
    /// Returns the exact trimmed command passed to the platform shell.
    #[must_use]
    pub fn command(&self) -> &str {
        self.approval_key.as_str()
    }

    /// Returns the exact command key a host must use for approval policy.
    #[must_use]
    pub fn approval_key(&self) -> &ExactCommandApproval {
        &self.approval_key
    }

    /// Returns a user-facing first executable token, not an approval identity.
    #[must_use]
    pub fn display_executable(&self) -> &str {
        &self.display_executable
    }

    /// Returns the bounded command timeout.
    #[must_use]
    pub fn timeout(&self) -> Duration {
        self.timeout
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellArgs {
    command: String,
    timeout_seconds: Option<u64>,
}

pub(crate) fn parse(input: &str) -> Result<PreparedShell, String> {
    let args = serde_json::from_str::<ShellArgs>(input)
        .map_err(|error| format!("invalid run_shell arguments: {error}"))?;
    let approval_key = ExactCommandApproval::new(args.command)?;
    let timeout_seconds = args
        .timeout_seconds
        .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
        .clamp(1, MAX_TIMEOUT_SECONDS);
    Ok(PreparedShell {
        display_executable: executable_token(approval_key.as_str())?,
        approval_key,
        timeout: Duration::from_secs(timeout_seconds),
    })
}

fn executable_token(command: &str) -> Result<String, String> {
    let command = command.trim_start();
    let token = if let Some(quote) = command
        .chars()
        .next()
        .filter(|character| matches!(*character, '"' | '\''))
    {
        let rest = &command[quote.len_utf8()..];
        let Some(end) = rest.find(quote) else {
            return Err("command begins with an unterminated quoted executable".into());
        };
        &rest[..end]
    } else {
        command.split_whitespace().next().unwrap_or_default()
    };
    if token.is_empty() {
        return Err("command must begin with an executable token".into());
    }
    #[cfg(windows)]
    {
        Ok(token.to_ascii_lowercase())
    }
    #[cfg(unix)]
    {
        Ok(token.into())
    }
}

pub(crate) enum ShellExecution {
    Completed(ToolExecution),
    Cancelled,
}

pub(crate) async fn execute(
    shell: &PreparedShell,
    workspace: &std::path::Path,
    cancellation: &CancellationToken,
    shutdown: &CancellationToken,
) -> ShellExecution {
    if cancellation.is_cancelled() || shutdown.is_cancelled() {
        return ShellExecution::Cancelled;
    }
    let mut command = platform_shell_command(shell.command());
    command
        .current_dir(workspace)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = match command.group().kill_on_drop(true).spawn() {
        Ok(child) => child,
        Err(error) => {
            return ShellExecution::Completed(ToolExecution::failed(format!(
                "could not start shell command: {error}"
            )))
        }
    };
    let capture = new_capture();
    let stdout_task = child
        .inner()
        .stdout
        .take()
        .map(|stream| start_capture(stream, capture.clone()));
    let stderr_task = child
        .inner()
        .stderr
        .take()
        .map(|stream| start_capture(stream, capture.clone()));
    let timer = tokio::time::sleep(shell.timeout());
    tokio::pin!(timer);
    let wait_result = {
        // Group waits include descendants, so wait for the leader here. This permits
        // the bounded pipe drain below before terminating any remaining group members.
        let wait = child.inner().wait();
        tokio::pin!(wait);
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => WaitResult::Cancelled,
            _ = shutdown.cancelled() => WaitResult::Cancelled,
            _ = &mut timer => WaitResult::TimedOut,
            status = &mut wait => WaitResult::Status(status),
        }
    };
    match wait_result {
        WaitResult::Status(Ok(status)) => {
            finish_capture(stdout_task, stderr_task).await;
            // A shell can exit while a descendant retains one of its pipes. Do not
            // let that descendant outlive this tool invocation.
            kill_child(&mut child).await;
            ShellExecution::Completed(status_result(status, &capture))
        }
        WaitResult::Status(Err(error)) => {
            kill_child(&mut child).await;
            abort_capture(stdout_task, stderr_task).await;
            ShellExecution::Completed(ToolExecution::failed(format!(
                "could not wait for shell command: {error}"
            )))
        }
        WaitResult::Cancelled => {
            kill_child(&mut child).await;
            abort_capture(stdout_task, stderr_task).await;
            ShellExecution::Cancelled
        }
        WaitResult::TimedOut => {
            kill_child(&mut child).await;
            abort_capture(stdout_task, stderr_task).await;
            ShellExecution::Completed(ToolExecution::failed(format!(
                "Command timed out after {} seconds and was terminated.\n{}",
                shell.timeout().as_secs(),
                captured_output(&capture)
            )))
        }
    }
}

enum WaitResult {
    Status(std::io::Result<std::process::ExitStatus>),
    Cancelled,
    TimedOut,
}

fn platform_shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let executable = std::env::var_os("ComSpec")
            .filter(|path| !path.is_empty())
            .unwrap_or_else(|| "cmd.exe".into());
        let mut shell = Command::new(executable);
        shell.args(["/D", "/S", "/C", command]);
        shell
    }
    #[cfg(unix)]
    {
        let mut shell = Command::new("/bin/sh");
        shell.args(["-c", command]);
        shell
    }
}

async fn kill_child(child: &mut AsyncGroupChild) {
    let _ = child.start_kill();
    let _ = child.wait().await;
}

fn status_result(status: std::process::ExitStatus, capture: &Capture) -> ToolExecution {
    let output = captured_output(capture);
    let code = status
        .code()
        .map_or_else(|| "terminated by signal".into(), |code| code.to_string());
    if status.success() {
        ToolExecution::succeeded(
            "Ran shell command",
            format!("Command exited successfully ({code}).\n{output}"),
        )
    } else {
        let mut result =
            ToolExecution::failed(format!("Command exited with status {code}.\n{output}"));
        result.past_tense_message = "Shell command exited unsuccessfully".into();
        result.error = Some(serde_json::json!({ "exitCode": status.code() }));
        result
    }
}
