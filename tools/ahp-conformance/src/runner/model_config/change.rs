use std::time::Duration;

use serde_json::{json, Value};
use uuid::Uuid;

use super::super::advanced::fixture_prompt;
use super::super::local_stack::LocalStack;
use super::super::{ConformanceError, Probe};
use super::helpers::{
    assert_action_model, assert_action_turn, assert_fixture_request, assert_session_config,
    complete_turn, create_session, dispatch_turn, model_selection, require_result, session_config,
    session_state, FAST_MODEL, REASONING_MODEL,
};
use super::session::initialized_probe;

pub(super) async fn session_model_change(
    target: &str,
    timeout: Duration,
    stack: &LocalStack,
) -> Result<String, ConformanceError> {
    let mut probe = initialized_probe(target, timeout, "session-model-change").await?;
    let initial = model_selection(REASONING_MODEL, Some("high"));
    let session = create_session(
        &mut probe,
        "model-change",
        session_config(REASONING_MODEL, Some("high")),
    )
    .await?;
    let fast = model_selection(FAST_MODEL, Some("low"));
    dispatch_config_change(&mut probe, &session.session, &fast).await?;
    let changed = probe
        .wait_action(&session.session, "session/configChanged")
        .await?;
    assert_accepted_change(&changed, &fast, 1)?;
    replay_change(target, timeout, &session.session, &fast).await?;
    let state = session_state(&mut probe, &session.session).await?;
    assert_session_config(&state, &fast)?;
    for invalid in [
        model_selection(FAST_MODEL, Some("high")),
        model_selection("not-in-fixture-catalog", Some("low")),
    ] {
        dispatch_config_change(&mut probe, &session.session, &invalid).await?;
        let rejected = probe
            .wait_action(&session.session, "session/configChanged")
            .await?;
        assert_rejected_change(&rejected)?;
    }
    let state = session_state(&mut probe, &session.session).await?;
    assert_session_config(&state, &fast)?;
    let first_request = stack.fixture_requests().len();
    let first = complete_turn(
        &mut probe,
        &session.chat,
        "session-model-change-idle-turn",
        fixture_prompt("changed session default", 1, 0),
        None,
    )
    .await?;
    assert_action_model(&first, &fast)?;
    assert_fixture_request(stack, first_request, FAST_MODEL, "low").await?;

    let active_request = stack.fixture_requests().len();
    dispatch_turn(
        &mut probe,
        &session.chat,
        "session-model-change-active-turn",
        fixture_prompt("active model remains fixed", 1, 300),
        None,
    )
    .await?;
    let active_started = probe.wait_action(&session.chat, "chat/turnStarted").await?;
    assert_action_turn(
        &active_started,
        "session-model-change-active-turn",
        "chat/turnStarted",
    )?;
    assert_action_model(&active_started, &fast)?;
    dispatch_config_change(&mut probe, &session.session, &initial).await?;
    let active_change = probe
        .wait_action(&session.session, "session/configChanged")
        .await?;
    assert_accepted_change(&active_change, &initial, 6)?;
    let active_terminal = probe
        .wait_action_any(&session.chat, &["chat/turnComplete", "chat/error"])
        .await?;
    assert_action_turn(
        &active_terminal,
        "session-model-change-active-turn",
        "active turn terminal",
    )?;
    if active_terminal
        .pointer("/params/action/type")
        .and_then(Value::as_str)
        != Some("chat/turnComplete")
    {
        return Err(ConformanceError::Options(
            "active-turn model mutation caused the active turn to fail".into(),
        ));
    }
    assert_fixture_request(stack, active_request, FAST_MODEL, "low").await?;
    let state = session_state(&mut probe, &session.session).await?;
    assert_session_config(&state, &initial)?;
    let future_request = stack.fixture_requests().len();
    let future = complete_turn(
        &mut probe,
        &session.chat,
        "session-model-change-future-turn",
        fixture_prompt("future model uses changed config", 1, 0),
        None,
    )
    .await?;
    assert_action_model(&future, &initial)?;
    assert_fixture_request(stack, future_request, REASONING_MODEL, "high").await?;
    probe.close().await?;
    Ok("replays accepted changes, rejects invalid values, and applies active changes only to future turns".into())
}

async fn dispatch_config_change(
    probe: &mut Probe,
    session: &str,
    selection: &Value,
) -> Result<(), ConformanceError> {
    probe
        .dispatch_action(
            session,
            json!({
                "type": "session/configChanged",
                "config": { "model": selection },
            }),
        )
        .await
}

fn assert_accepted_change(
    action: &Value,
    selection: &Value,
    client_seq: i64,
) -> Result<(), ConformanceError> {
    if action
        .get("params")
        .and_then(|params| params.get("rejectionReason"))
        .is_some()
        || action
            .pointer("/params/origin/clientSeq")
            .and_then(Value::as_i64)
            != Some(client_seq)
        || action.pointer("/params/action/config/model") != Some(selection)
    {
        return Err(ConformanceError::Options(
            "session/configChanged was not an authoritative accepted action".into(),
        ));
    }
    Ok(())
}

fn assert_rejected_change(action: &Value) -> Result<(), ConformanceError> {
    if action
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
    {
        Ok(())
    } else {
        Err(ConformanceError::Options(
            "invalid session/configChanged was not rejected authoritatively".into(),
        ))
    }
}

async fn replay_change(
    target: &str,
    timeout: Duration,
    session: &str,
    selection: &Value,
) -> Result<(), ConformanceError> {
    let mut replay = Probe::connect(target, timeout).await?;
    let reply = replay
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": format!("session-model-change-replay-{}", Uuid::new_v4()),
                "lastSeenServerSeq": 0,
                "subscriptions": ["ahp-root://", session],
            }),
        )
        .await?;
    let actions = require_result(&reply, "reconnect")?
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ConformanceError::Options("reconnect did not return action replay".into())
        })?;
    if !actions.iter().any(|action| {
        action.get("channel").and_then(Value::as_str) == Some(session)
            && action.pointer("/action/type").and_then(Value::as_str)
                == Some("session/configChanged")
            && action.pointer("/action/config/model") == Some(selection)
            && action.pointer("/origin/clientSeq").and_then(Value::as_i64) == Some(1)
    }) {
        return Err(ConformanceError::Options(
            "reconnect did not replay the authoritative config change".into(),
        ));
    }
    replay.close().await
}
