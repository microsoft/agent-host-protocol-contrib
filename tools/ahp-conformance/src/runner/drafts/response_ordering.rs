//! Tool-to-text response-part ordering through live delivery and replay.

use std::path::Path;
use std::time::Duration;

use serde_json::json;
use uuid::Uuid;

use super::super::advanced::start_turn;
use super::super::tools::workspace::{file_uri, workspace_chat};
use super::super::{ConformanceError, Probe};
use super::ordering::{
    assert_ordered_lifecycle, collect_turn_actions, replay_actions, same_actions, FINAL_RESPONSE,
    TURN_ID,
};
use super::ordering_snapshot::assert_snapshot_order;

pub(in crate::runner) async fn response_part_ordering(
    target: &str,
    timeout: Duration,
    workspace: &Path,
) -> Result<String, ConformanceError> {
    let file_name = "response-part-ordering.txt";
    std::fs::write(workspace.join(file_name), "before OLD after")?;
    let mut probe = Probe::connect(target, timeout).await?;
    let session =
        workspace_chat(&mut probe, "response-part-ordering", &file_uri(workspace)?).await?;
    start_turn(
        &mut probe,
        &session.chat,
        TURN_ID,
        fixture_prompt(file_name),
    )
    .await?;
    let live = collect_turn_actions(&mut probe, &session.chat).await?;
    assert_ordered_lifecycle(&live)?;
    let snapshot = super::helpers::chat_state(&mut probe, &session.chat).await?;
    assert_snapshot_order(&snapshot, &live)?;
    probe.close().await?;

    let mut reconnected = Probe::connect(target, timeout).await?;
    let replay = reconnected
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": format!("response-part-ordering-replay-{}", Uuid::new_v4()),
                "lastSeenServerSeq": 0,
                "subscriptions": [&session.chat],
            }),
        )
        .await?;
    let replayed = replay_actions(&replay, &session.chat)?;
    assert_ordered_lifecycle(&replayed)?;
    if !same_actions(&live, &replayed) {
        return Err(ConformanceError::Options(
            "replay did not preserve the live response-part action sequence exactly".to_owned(),
        ));
    }
    reconnected.close().await?;
    Ok(
        "orders v2 tool lifecycle before a distinct final markdown response part in live and replay state"
            .to_owned(),
    )
}

fn fixture_prompt(file_name: &str) -> String {
    format!(
        "[AHP-TEST v2]\nCall tool replace_string with JSON: {{\"path\":\"{file_name}\",\"old_string\":\"OLD\",\"new_string\":\"NEW\"}}\nRespond exactly after tools with JSON: \"{FINAL_RESPONSE}\".\n[/AHP-TEST]"
    )
}
