//! Streaming-cancellation assertions shared by explicit upstream API cases.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use super::super::tools::helpers::require_turn;
use super::super::tools::workspace::{file_uri, workspace_chat};
use super::super::turns::assert_cancelled_snapshot;
use super::super::{ConformanceError, Probe};
use super::text::start_text_turn;

const CANCELLATION_CHUNKS: usize = 4;
const CANCELLATION_DELAY_MS: u64 = 200;
const QUIESCENCE: Duration = Duration::from_secs(1);

pub(super) async fn assert_streaming_cancellation(
    target: &str,
    timeout: Duration,
    workspace: &Path,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, suffix, &file_uri(workspace)?).await?;
    let turn_id = format!("{suffix}-cancel");
    start_text_turn(
        &mut probe,
        &session.chat,
        &turn_id,
        "cancel this true streaming response",
        CANCELLATION_CHUNKS,
        CANCELLATION_DELAY_MS,
    )
    .await?;
    loop {
        let delta = probe.wait_action(&session.chat, "chat/delta").await?;
        require_turn(&delta, &turn_id, "chat/delta")?;
        if delta
            .pointer("/params/action/content")
            .and_then(Value::as_str)
            .is_some_and(|content| !content.is_empty())
        {
            break;
        }
    }
    probe
        .dispatch_action(
            &session.chat,
            serde_json::json!({ "type": "chat/turnCancelled", "turnId": &turn_id }),
        )
        .await?;
    let cancelled = probe
        .wait_action(&session.chat, "chat/turnCancelled")
        .await?;
    require_turn(&cancelled, &turn_id, "chat/turnCancelled")?;
    let cancelled_sequence = Probe::action_server_sequence(&cancelled)?;
    probe
        .assert_no_post_cancel_activity_for_turn(
            &session.chat,
            &turn_id,
            cancelled_sequence,
            QUIESCENCE,
        )
        .await?;
    let snapshot = probe
        .request("subscribe", serde_json::json!({ "channel": &session.chat }))
        .await?
        .pointer("/result/snapshot/state")
        .cloned()
        .ok_or_else(|| {
            ConformanceError::Options("stream cancellation omitted a chat snapshot".to_owned())
        })?;
    assert_cancelled_snapshot(&snapshot, &turn_id)?;
    probe.close().await
}
