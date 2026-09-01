//! Local workspace-selection conformance case and chat setup helpers.

use std::path::Path;
use std::time::Duration;

use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::{ConformanceError, Probe};

pub(in super::super) struct WorkspaceChat {
    pub(in super::super) session: String,
    pub(in super::super) chat: String,
}

pub(in super::super) fn file_uri(path: &Path) -> Result<String, ConformanceError> {
    reqwest::Url::from_file_path(path)
        .map(|uri| uri.to_string())
        .map_err(|()| ConformanceError::Options(format!("could not encode file URI for {path:?}")))
}

pub(in super::super) async fn workspace_chat(
    probe: &mut Probe,
    suffix: &str,
    workspace_uri: &str,
) -> Result<WorkspaceChat, ConformanceError> {
    probe
        .initialize(
            &format!("{suffix}-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let session = format!("ahp-session:/{}", Uuid::new_v4());
    let created = probe
        .request(
            "createSession",
            json!({
                "channel": session,
                "provider": "openai",
                "workingDirectories": [workspace_uri],
            }),
        )
        .await?;
    if created.get("error").is_some() {
        return Err(ConformanceError::Options(
            "workspace createSession returned an unexpected JSON-RPC error".to_owned(),
        ));
    }
    let snapshot = probe
        .request("subscribe", json!({ "channel": &session }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("session snapshot was omitted".to_owned()))?;
    let chat = snapshot
        .pointer("/chats/0/resource")
        .and_then(Value::as_str)
        .filter(|chat| chat.starts_with("ahp-chat:/"))
        .ok_or_else(|| ConformanceError::Options("session has no default chat".to_owned()))?
        .to_owned();
    let subscribed = probe
        .request("subscribe", json!({ "channel": &chat }))
        .await?;
    if subscribed.get("error").is_some() {
        return Err(ConformanceError::Options(
            "chat subscribe returned an unexpected JSON-RPC error".to_owned(),
        ));
    }
    Ok(WorkspaceChat { session, chat })
}

pub(in super::super) async fn workspace_validation(
    target: &str,
    timeout: Duration,
    allowed_root: &Path,
    selected_workspace: &Path,
    outside_workspace: &Path,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let initialized = probe
        .initialize(
            &format!("workspace-validation-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    if initialized.get("error").is_some() {
        return Err(ConformanceError::Options(
            "workspace-validation client could not initialize".to_owned(),
        ));
    }
    for (label, workspace) in [
        (
            "non-file URI",
            "https://example.test/not-a-workspace".to_owned(),
        ),
        (
            "nonexistent directory",
            file_uri(&allowed_root.join("does-not-exist"))?,
        ),
        ("outside allowed root", file_uri(outside_workspace)?),
    ] {
        let rejected = probe
            .request(
                "createSession",
                json!({
                    "channel": format!("ahp-session:/{}", Uuid::new_v4()),
                    "provider": "openai",
                    "workingDirectories": [workspace],
                }),
            )
            .await?;
        if rejected.get("error").is_none() {
            return Err(ConformanceError::Options(format!(
                "createSession accepted {label}"
            )));
        }
    }
    let expected_workspace = file_uri(&selected_workspace.canonicalize()?)?;
    let workspace_chat =
        workspace_chat(&mut probe, "workspace-accepted", &expected_workspace).await?;
    let snapshot = probe
        .request("subscribe", json!({ "channel": workspace_chat.session }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| {
            ConformanceError::Options("accepted workspace omitted snapshot".to_owned())
        })?;
    let workspaces = snapshot
        .get("workingDirectories")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ConformanceError::Options("session omitted workingDirectories".to_owned())
        })?;
    if workspaces.len() != 1
        || workspaces.first().and_then(Value::as_str) != Some(expected_workspace.as_str())
    {
        return Err(ConformanceError::Options(
            "session did not report exactly the selected canonical file URI".to_owned(),
        ));
    }
    let tools = snapshot
        .get("serverTools")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("session omitted serverTools".to_owned()))?;
    let names = tools
        .iter()
        .filter_map(|tool| tool.get("name").and_then(Value::as_str))
        .collect::<Vec<_>>();
    if names != ["replace_string", "run_shell"] {
        return Err(ConformanceError::Options(format!(
            "serverTools were {names:?}, expected exactly replace_string and run_shell"
        )));
    }
    if tools.iter().any(|tool| {
        tool.get("inputSchema")
            .filter(|schema| schema.is_object())
            .and_then(|schema| schema.get("type"))
            .and_then(Value::as_str)
            != Some("object")
    }) {
        return Err(ConformanceError::Options(
            "serverTools did not expose object input schemas".to_owned(),
        ));
    }
    probe.close().await?;
    Ok("rejects invalid workspaces and advertises canonical workspace-scoped tools".to_owned())
}
