mod common;

use std::fs;
use std::process::Command;
use std::time::{Duration, Instant};

use common::{workspace_tools, TestDir};
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{
    ExecutionOutcome, ToolInvocation, MAX_OUTPUT_BYTES, MAX_TIMEOUT_SECONDS,
};

#[test]
fn shell_preflight_exposes_exact_command_identity_without_executing(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("shell-identity")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(r#"{"command":"  echo first  ","timeout_seconds":999}"#),
    )?;
    let Some(shell) = prepared.shell() else {
        return Err("run_shell did not prepare a shell command".into());
    };
    assert_eq!(shell.command(), "echo first");
    assert_eq!(shell.approval_key().as_str(), "echo first");
    assert_eq!(shell.timeout().as_secs(), MAX_TIMEOUT_SECONDS);
    assert_eq!(shell.display_executable(), "echo");
    assert!(shell.approval_key().matches_command("  echo first  "));
    assert!(!shell.approval_key().matches_command("echo second"));
    assert!(!shell.approval_key().matches_command("ECHO first"));
    let not_run = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(r#"{"command":"echo prepared > preflight-marker.txt"}"#),
    )?;
    drop(not_run);
    assert!(!root.path().join("preflight-marker.txt").exists());
    Ok(())
}

#[tokio::test]
async fn shell_cancellation_terminates_execution() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("shell-cancel")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    #[cfg(windows)]
    let command = "ping -n 30 127.0.0.1 > NUL";
    #[cfg(unix)]
    let command = "sleep 30";
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(serde_json::json!({ "command": command }).to_string()),
    )?;
    let cancellation = CancellationToken::new();
    let cancellation_task = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancellation_task.cancel();
    });
    let shutdown = CancellationToken::new();
    assert!(matches!(
        tools
            .execute(&workspace, prepared, &cancellation, &shutdown)
            .await,
        ExecutionOutcome::Cancelled
    ));
    Ok(())
}

#[tokio::test]
async fn shell_output_is_limited_to_64_kib() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("shell-output")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    #[cfg(windows)]
    let command = {
        fs::write(
            root.path().join("output.ps1"),
            "[Console]::Out.Write(('x' * 70000))",
        )?;
        "powershell.exe -NoProfile -File output.ps1"
    };
    #[cfg(unix)]
    let command = "yes x | head -c 70000";
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(serde_json::json!({ "command": command }).to_string()),
    )?;
    let live = CancellationToken::new();
    let outcome = tools.execute(&workspace, prepared, &live, &live).await;
    let ExecutionOutcome::Completed(result) = outcome else {
        return Err("large-output command was unexpectedly cancelled".into());
    };
    let output = result.model_content();
    assert!(result.success);
    assert!(output.contains("[output truncated at 65536 bytes]"));
    assert!(output.len() <= MAX_OUTPUT_BYTES + 128);
    Ok(())
}

#[tokio::test]
async fn shell_does_not_wait_for_a_retained_pipe_after_parent_exit(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("shell-pipe")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    let retained_marker = root.path().join("retained-child.txt");
    #[cfg(windows)]
    let command = {
        fs::write(
            root.path().join("retain-pipe.ps1"),
            "$psi = [System.Diagnostics.ProcessStartInfo]::new()\n\
             $psi.FileName = 'cmd.exe'\n\
             $psi.Arguments = '/D /S /C ping -n 3 127.0.0.1 > NUL & echo leaked > retained-child.txt'\n\
             $psi.UseShellExecute = $false\n\
             [System.Diagnostics.Process]::Start($psi) | Out-Null\n\
             Write-Output done",
        )?;
        "powershell.exe -NoProfile -File retain-pipe.ps1"
    };
    #[cfg(unix)]
    let command = "(sleep 2; printf leaked > retained-child.txt) & echo done";
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(serde_json::json!({ "command": command }).to_string()),
    )?;
    let live = CancellationToken::new();
    let started = Instant::now();
    let outcome = tools.execute(&workspace, prepared, &live, &live).await;
    assert!(matches!(outcome, ExecutionOutcome::Completed(_)));
    assert!(started.elapsed() < Duration::from_secs(4));
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(
        !retained_marker.exists(),
        "a descendant outlived the completed shell invocation"
    );
    Ok(())
}

#[tokio::test]
async fn timeout_terminates_a_descendant_process() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("shell-descendant")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    #[cfg(windows)]
    let command = {
        fs::write(
            root.path().join("child.ps1"),
            "$p = Start-Process -FilePath 'cmd.exe' -ArgumentList '/D /S /C ping -n 30 127.0.0.1' -PassThru\n\
             [System.IO.File]::WriteAllText('child.pid', $p.Id)\n\
             Start-Sleep -Seconds 30",
        )?;
        "powershell.exe -NoProfile -File child.ps1"
    };
    #[cfg(unix)]
    let command = "sleep 30 & echo $! > child.pid; wait";
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::run_shell(
            serde_json::json!({ "command": command, "timeout_seconds": 3 }).to_string(),
        ),
    )?;
    let live = CancellationToken::new();
    let outcome = tools.execute(&workspace, prepared, &live, &live).await;
    assert!(matches!(
        outcome,
        ExecutionOutcome::Completed(result) if !result.success
    ));
    let pid = wait_for_pid(&root.path().join("child.pid")).await?;
    wait_until_dead(pid).await?;
    Ok(())
}

async fn wait_for_pid(path: &std::path::Path) -> Result<u32, Box<dyn std::error::Error>> {
    for _ in 0..20 {
        if let Ok(value) = fs::read_to_string(path) {
            return Ok(value.trim().parse()?);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    Err(format!("did not receive child PID at {}", path.display()).into())
}

async fn wait_until_dead(pid: u32) -> Result<(), Box<dyn std::error::Error>> {
    for _ in 0..30 {
        if !process_exists(pid)? {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(format!("descendant process {pid} still exists after termination").into())
}

fn process_exists(pid: u32) -> Result<bool, std::io::Error> {
    #[cfg(windows)]
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "if (Get-Process -Id {pid} -ErrorAction SilentlyContinue) {{ exit 0 }} else {{ exit 1 }}"
            ),
        ])
        .status()?;
    #[cfg(unix)]
    let status = Command::new("/bin/sh")
        .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
        .status()?;
    Ok(status.success())
}
