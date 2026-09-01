//! Black-box function-argument streaming assertions for both upstream APIs.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::super::tools::helpers::{
    action_value, await_tool_complete, require_turn, start_fixture_tool_turn_with_delay,
};
use super::super::tools::lifecycle::complete_fixture_turn;
use super::super::tools::workspace::{file_uri, workspace_chat};
use super::super::{ConformanceError, Probe};

const REPLACEMENT: &str = "streamed-tool-argument-value-0123456789";
const FINAL_RESPONSE: &str = "streamed tool call completed";
const ARGUMENT_DELAY_MS: u64 = 200;

pub(super) async fn assert_tool_streaming(
    target: &str,
    timeout: Duration,
    workspace: &Path,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let path = workspace.join("stream-tool.txt");
    std::fs::write(&path, "before OLD after")?;
    let arguments = json!({
        "path": "stream-tool.txt",
        "old_string": "OLD",
        "new_string": REPLACEMENT,
    });
    let expected_arguments =
        format!(r#"{{"new_string":"{REPLACEMENT}","old_string":"OLD","path":"stream-tool.txt"}}"#);
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(&mut probe, suffix, &file_uri(workspace)?).await?;
    let turn_id = format!("{suffix}-tool");
    let started = Instant::now();
    let call = start_fixture_tool_turn_with_delay(
        &mut probe,
        &session.chat,
        &turn_id,
        "replace_string",
        arguments,
        FINAL_RESPONSE,
        ARGUMENT_DELAY_MS,
    )
    .await?;
    let mut received = String::new();
    loop {
        let event = probe
            .wait_action_any(
                &session.chat,
                &["chat/toolCallDelta", "chat/toolCallReady", "chat/error"],
            )
            .await?;
        let action = action_value(&event)?;
        match action.get("type").and_then(Value::as_str) {
            Some("chat/toolCallDelta") => {
                require_turn(&event, &turn_id, "chat/toolCallDelta")?;
                if action.get("toolCallId").and_then(Value::as_str) != Some(call.id.as_str()) {
                    return Err(ConformanceError::Options(
                        "tool argument delta targeted a different call".to_owned(),
                    ));
                }
                let first_delta = received.is_empty();
                let content = action
                    .get("content")
                    .and_then(Value::as_str)
                    .filter(|content| !content.is_empty())
                    .ok_or_else(|| {
                        ConformanceError::Options(
                            "tool argument delta omitted nonempty content".to_owned(),
                        )
                    })?;
                received.push_str(content);
                if first_delta && started.elapsed() >= Duration::from_millis(ARGUMENT_DELAY_MS * 2)
                {
                    return Err(ConformanceError::Options(
                        "first tool argument delta arrived no earlier than the full fixture delay"
                            .to_owned(),
                    ));
                }
            }
            Some("chat/toolCallReady") => {
                require_turn(&event, &turn_id, "chat/toolCallReady")?;
                if action.get("toolCallId").and_then(Value::as_str) != Some(call.id.as_str())
                    || action.get("confirmed").and_then(Value::as_str) != Some("not-needed")
                    || action
                        .get("options")
                        .is_some_and(|options| !options.is_null())
                {
                    return Err(ConformanceError::Options(
                        "toolCallReady did not follow streamed automatic tool input".to_owned(),
                    ));
                }
                break;
            }
            Some("chat/error") => {
                return Err(ConformanceError::Options(
                    "streaming tool turn ended in chat/error".to_owned(),
                ))
            }
            _ => {
                return Err(ConformanceError::Options(
                    "unexpected tool stream event".to_owned(),
                ))
            }
        }
    }
    if received.is_empty() || received != expected_arguments {
        return Err(ConformanceError::Options(format!(
            "tool argument deltas reconstructed {received:?}, expected {expected_arguments:?}"
        )));
    }
    let result = await_tool_complete(&mut probe, &session.chat, &turn_id, &call).await?;
    if !result.success {
        return Err(ConformanceError::Options(
            "streamed replace_string tool did not succeed".to_owned(),
        ));
    }
    complete_fixture_turn(&mut probe, &session.chat, &turn_id, FINAL_RESPONSE).await?;
    if std::fs::read_to_string(path)? != format!("before {REPLACEMENT} after") {
        return Err(ConformanceError::Options(
            "streamed tool call did not apply its reconstructed input".to_owned(),
        ));
    }
    probe.close().await
}
