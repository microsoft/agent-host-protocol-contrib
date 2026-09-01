use std::time::Duration;

use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::advanced::fixture_prompt;
use super::super::local_stack::LocalStack;
use super::super::{ConformanceError, Probe};
use super::helpers::{
    assert_action_model, assert_fixture_request, assert_historical_model, assert_session_config,
    chat_state, complete_turn, create_session, model_selection, require_result, session_config,
    session_state, FAST_MODEL, REASONING_MODEL,
};

pub(super) async fn session_model_default(
    target: &str,
    timeout: Duration,
    stack: &LocalStack,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "session-model-default").await?;
    let selected = model_selection(REASONING_MODEL, Some("high"));
    let resolved = probe
        .request(
            "resolveSessionConfig",
            json!({
                "channel": "ahp-root://",
                "provider": "openai",
                "config": { "model": selected },
            }),
        )
        .await?;
    let config = require_result(&resolved, "resolveSessionConfig")?
        .get("values")
        .cloned()
        .ok_or_else(|| ConformanceError::Options("resolved config omitted values".into()))?;
    let session = create_session(&mut probe, "model-default", config).await?;
    let state = session_state(&mut probe, &session.session).await?;
    assert_session_config(&state, &selected)?;
    let request_index = stack.fixture_requests().len();
    let started = complete_turn(
        &mut probe,
        &session.chat,
        "session-model-default-turn",
        fixture_prompt("session model default", 1, 0),
        None,
    )
    .await?;
    assert_action_model(&started, &selected)?;
    let history = chat_state(&mut probe, &session.chat).await?;
    assert_historical_model(&history, "session-model-default-turn", &selected)?;
    assert_fixture_request(stack, request_index, REASONING_MODEL, "high").await?;
    probe.close().await?;
    Ok("persists a resolved session model and applies it to model-less turns".into())
}

pub(super) async fn message_model_override(
    target: &str,
    timeout: Duration,
    stack: &LocalStack,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "message-model-override").await?;
    let default = model_selection(FAST_MODEL, Some("low"));
    let override_model = model_selection(REASONING_MODEL, Some("xhigh"));
    let session = create_session(
        &mut probe,
        "message-model-override",
        session_config(FAST_MODEL, Some("low")),
    )
    .await?;
    let request = stack.fixture_requests().len();
    let started = complete_turn(
        &mut probe,
        &session.chat,
        "message-model-override-turn",
        fixture_prompt("message model override", 1, 0),
        Some(override_model.clone()),
    )
    .await?;
    assert_action_model(&started, &override_model)?;
    let history = chat_state(&mut probe, &session.chat).await?;
    assert_historical_model(&history, "message-model-override-turn", &override_model)?;
    assert_fixture_request(stack, request, REASONING_MODEL, "xhigh").await?;
    let state = session_state(&mut probe, &session.session).await?;
    assert_session_config(&state, &default)?;
    let next_request = stack.fixture_requests().len();
    let next = complete_turn(
        &mut probe,
        &session.chat,
        "message-model-default-followup",
        fixture_prompt("message default followup", 1, 0),
        None,
    )
    .await?;
    assert_action_model(&next, &default)?;
    assert_fixture_request(stack, next_request, FAST_MODEL, "low").await?;
    probe.close().await?;
    Ok("uses a message model override without changing the following session default".into())
}

pub(super) async fn initialized_probe(
    target: &str,
    timeout: Duration,
    name: &str,
) -> Result<Probe, ConformanceError> {
    let mut probe = Probe::connect(target, timeout).await?;
    let initialized = probe
        .initialize(
            &format!("{name}-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    require_result(&initialized, "initialize")?;
    Ok(probe)
}
