//! Black-box general session persistence probe for the local sample stack.

use std::path::Path;
use std::time::Duration;

use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::advanced::{fixture_prompt, start_turn};
use super::super::{ConformanceError, Probe};
use super::workspace::{file_uri, workspace_chat, WorkspaceChat};

pub(in super::super) async fn create_persisted_turn(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<WorkspaceChat, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, "persistence-restart", &file_uri(workspace)?).await?;
    start_turn(
        &mut probe,
        &session.chat,
        "persistence-restart-turn",
        fixture_prompt("durable completed response", 2, 0),
    )
    .await?;
    probe
        .wait_action(&session.chat, "chat/turnComplete")
        .await?;
    probe.close().await?;
    Ok(session)
}

pub(in super::super) async fn verify_persisted_turn(
    target: &str,
    timeout: Duration,
    retained: &WorkspaceChat,
) -> Result<(), ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let initialized = probe
        .initialize(
            &format!("persistence-reader-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let previous = initialized
        .pointer("/result/serverSeq")
        .and_then(Value::as_i64)
        .filter(|sequence| *sequence > 0)
        .ok_or_else(|| ConformanceError::Options("restart did not retain serverSeq".to_owned()))?;
    let listed = probe
        .request("listSessions", json!({ "channel": "ahp-root://" }))
        .await?;
    if !listed
        .pointer("/result/items")
        .and_then(Value::as_array)
        .is_some_and(|items| {
            items.iter().any(|item| {
                item.get("resource").and_then(Value::as_str) == Some(retained.session.as_str())
            })
        })
    {
        return Err(ConformanceError::Options(
            "created session was not retained across restart".to_owned(),
        ));
    }
    let snapshot = probe
        .request("subscribe", json!({ "channel": &retained.chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("restart chat snapshot was omitted".to_owned()))?;
    if !snapshot
        .get("turns")
        .and_then(Value::as_array)
        .is_some_and(|turns| {
            turns.iter().any(|turn| {
                turn.get("id").and_then(Value::as_str) == Some("persistence-restart-turn")
                    && turn.get("state").and_then(Value::as_str) == Some("complete")
            })
        })
    {
        return Err(ConformanceError::Options(
            "completed turn was not retained across restart".to_owned(),
        ));
    }
    probe
        .request(
            "createSession",
            json!({
                "channel": format!("ahp-session:/{}", Uuid::new_v4()),
                "provider": "openai",
            }),
        )
        .await?;
    let next = probe
        .wait_action("ahp-root://", "root/activeSessionsChanged")
        .await?;
    if next.pointer("/params/serverSeq").and_then(Value::as_i64) <= Some(previous) {
        return Err(ConformanceError::Options(
            "server sequence did not increase after restart".to_owned(),
        ));
    }
    probe.close().await
}
