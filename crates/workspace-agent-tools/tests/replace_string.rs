mod common;

use std::fs;

use ahp_types::state::ToolResultContent;
use common::{workspace_tools, TestDir};
use tokio_util::sync::CancellationToken;
use workspace_agent_tools::{ExecutionOutcome, ToolInvocation, MAX_REPLACE_FILE_BYTES};

#[tokio::test]
async fn replaces_once_and_converts_to_ahp_result() -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("replace-success")?;
    fs::write(root.path().join("sample.txt"), "before OLD after")?;
    let (tools, workspace) = workspace_tools(root.path())?;
    let prepared = tools.preflight(
        &workspace,
        ToolInvocation::replace_string(
            r#"{"path":"sample.txt","old_string":"OLD","new_string":"new"}"#,
        ),
    )?;
    let live = CancellationToken::new();
    let result = tools.execute(&workspace, prepared, &live, &live).await;
    let ExecutionOutcome::Completed(result) = result else {
        return Err("replacement was unexpectedly cancelled".into());
    };
    assert!(result.success);
    assert_eq!(
        fs::read_to_string(root.path().join("sample.txt"))?,
        "before new after"
    );
    let edit = result
        .content
        .iter()
        .find_map(|content| match content {
            ToolResultContent::FileEdit(edit) => Some(edit),
            _ => None,
        })
        .ok_or("replacement did not return a file edit")?;
    let after = edit.after.as_ref().ok_or("file edit has no after side")?;
    let expected_uri = url::Url::from_file_path(workspace.path().join("sample.txt"))
        .map_err(|()| "invalid test file URI")?;
    assert_eq!(after.uri, expected_uri.as_str());
    assert_eq!(after.content.uri, after.uri);
    assert_eq!(after.content.size_hint, Some(16));
    assert_eq!(after.content.content_type.as_deref(), Some("text/plain"));
    assert!(edit.before.is_none());
    assert!(edit.diff.is_none());
    let ahp = result.to_call_result();
    assert!(ahp.success);
    Ok(())
}

#[tokio::test]
async fn rejects_invalid_missing_non_unique_large_and_non_utf8_files(
) -> Result<(), Box<dyn std::error::Error>> {
    let root = TestDir::new("replace-failures")?;
    fs::write(root.path().join("sample.txt"), "same same")?;
    fs::write(root.path().join("binary.bin"), [0xff, 0xfe])?;
    fs::write(
        root.path().join("large.txt"),
        "x".repeat((MAX_REPLACE_FILE_BYTES + 1) as usize),
    )?;
    let (tools, workspace) = workspace_tools(root.path())?;
    let invalid = tools.preflight(
        &workspace,
        ToolInvocation::replace_string(r#"{"path":"sample.txt","old_string":"x"}"#),
    );
    assert!(invalid.is_err());
    let live = CancellationToken::new();
    for input in [
        r#"{"path":"missing.txt","old_string":"x","new_string":"y"}"#,
        r#"{"path":"sample.txt","old_string":"same","new_string":"x"}"#,
        r#"{"path":"large.txt","old_string":"x","new_string":"y"}"#,
        r#"{"path":"binary.bin","old_string":"x","new_string":"y"}"#,
    ] {
        let prepared = tools.preflight(&workspace, ToolInvocation::replace_string(input))?;
        let result = tools.execute(&workspace, prepared, &live, &live).await;
        assert!(matches!(result, ExecutionOutcome::Completed(result) if !result.success));
    }
    Ok(())
}
