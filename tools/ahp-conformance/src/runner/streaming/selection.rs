//! Isolated API-selection orchestration for true-upstream-streaming probes.

use std::path::PathBuf;
use std::time::Duration;

use openai_agent_model::ModelApi;
use serde_json::Value;

use super::super::advanced::{fixture_prompt, start_turn};
use super::super::local_stack::{FixtureSupport, LocalStack, LocalStreamingHost};
use super::super::tools::helpers::require_turn;
use super::super::tools::workspace::{file_uri, workspace_chat};
use super::super::{ConformanceError, Probe};
use super::cancel::assert_streaming_cancellation;
use super::text::assert_text_streaming;
use super::tools::assert_tool_streaming;

pub(in super::super) async fn responses_streaming(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    run_text_and_tools(
        stack,
        timeout,
        ModelApi::Responses,
        FixtureSupport::Dual,
        "responses",
    )
    .await?;
    Ok(
        "Responses emits a text delta before the 1000 ms fixture aggregate delay and streams tool arguments"
            .to_owned(),
    )
}

pub(in super::super) async fn chat_completions_streaming(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    run_text_and_tools(
        stack,
        timeout,
        ModelApi::ChatCompletions,
        FixtureSupport::Dual,
        "chat-completions",
    )
    .await?;
    Ok(
        "Chat Completions emits a text delta before the 1000 ms fixture aggregate delay and streams tool arguments"
            .to_owned(),
    )
}

pub(in super::super) async fn api_auto_selection(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    run_text(
        stack,
        timeout,
        ModelApi::Auto,
        FixtureSupport::Dual,
        "auto-dual",
    )
    .await?;
    run_text_and_tools(
        stack,
        timeout,
        ModelApi::Auto,
        FixtureSupport::ChatOnly,
        "auto-chat-only",
    )
    .await?;
    explicit_responses_rejects_chat_only(stack, timeout).await?;
    run_text(
        stack,
        timeout,
        ModelApi::ChatCompletions,
        FixtureSupport::ChatOnly,
        "explicit-chat-only",
    )
    .await?;
    Ok(
        "Auto works with dual and chat-only fixtures; explicit Responses errors and explicit Chat works"
            .to_owned(),
    )
}

pub(in super::super) async fn streaming_cancellation(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    run_cancellation(
        stack,
        timeout,
        ModelApi::Responses,
        "responses-stream-cancel",
    )
    .await?;
    run_cancellation(
        stack,
        timeout,
        ModelApi::ChatCompletions,
        "chat-stream-cancel",
    )
    .await?;
    Ok(
        "both explicit APIs cancel after their first streamed AHP delta without later activity"
            .to_owned(),
    )
}

async fn run_text_and_tools(
    stack: &LocalStack,
    timeout: Duration,
    api: ModelApi,
    support: FixtureSupport,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let host = stack.start_streaming_host(api, support).await?;
    let target = host.websocket_url();
    let workspace = host.workspace().to_path_buf();
    let result = async {
        assert_text_streaming(&target, timeout, &workspace, suffix).await?;
        assert_tool_streaming(&target, timeout, &workspace, suffix).await
    }
    .await;
    finish(host, result).await
}

async fn run_text(
    stack: &LocalStack,
    timeout: Duration,
    api: ModelApi,
    support: FixtureSupport,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let host = stack.start_streaming_host(api, support).await?;
    let target = host.websocket_url();
    let workspace = host.workspace().to_path_buf();
    finish(
        host,
        assert_text_streaming(&target, timeout, &workspace, suffix).await,
    )
    .await
}

async fn run_cancellation(
    stack: &LocalStack,
    timeout: Duration,
    api: ModelApi,
    suffix: &str,
) -> Result<(), ConformanceError> {
    let host = stack
        .start_streaming_host(api, FixtureSupport::Dual)
        .await?;
    let target = host.websocket_url();
    let workspace = host.workspace().to_path_buf();
    finish(
        host,
        assert_streaming_cancellation(&target, timeout, &workspace, suffix).await,
    )
    .await
}

async fn explicit_responses_rejects_chat_only(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<(), ConformanceError> {
    let host = stack
        .start_streaming_host(ModelApi::Responses, FixtureSupport::ChatOnly)
        .await?;
    let target = host.websocket_url();
    let workspace = host.workspace().to_path_buf();
    let result = explicit_responses_error(&target, timeout, workspace).await;
    finish(host, result).await
}

async fn explicit_responses_error(
    target: &str,
    timeout: Duration,
    workspace: PathBuf,
) -> Result<(), ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let session = workspace_chat(
        &mut probe,
        "explicit-responses-error",
        &file_uri(&workspace)?,
    )
    .await?;
    let turn_id = "explicit-responses-error";
    start_turn(
        &mut probe,
        &session.chat,
        turn_id,
        fixture_prompt("responses must not fall back", 3, 0),
    )
    .await?;
    let error = probe.wait_action(&session.chat, "chat/error").await?;
    require_turn(&error, turn_id, "chat/error")?;
    if error
        .pointer("/params/action/error/errorType")
        .and_then(Value::as_str)
        != Some("openai_upstream_error")
    {
        return Err(ConformanceError::Options(
            "explicit Responses against chat-only fixture did not surface chat/error".to_owned(),
        ));
    }
    probe.close().await
}

async fn finish<T>(
    host: LocalStreamingHost,
    result: Result<T, ConformanceError>,
) -> Result<T, ConformanceError> {
    let stopped = host.shutdown().await;
    match (result, stopped) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) | (_, Err(error)) => Err(error),
    }
}
