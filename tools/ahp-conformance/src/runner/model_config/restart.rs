use std::time::Duration;

use super::super::advanced::fixture_prompt;
use super::super::local_stack::LocalStack;
use super::super::ConformanceError;
use super::helpers::{
    assert_action_model, assert_fixture_request, assert_historical_model, assert_session_config,
    chat_state, complete_turn, create_session, model_selection, session_config, session_state,
    REASONING_MODEL,
};
use super::session::initialized_probe;

pub(super) async fn session_model_restart(
    stack: &LocalStack,
    timeout: Duration,
) -> Result<String, ConformanceError> {
    let first = stack.start_host().await?;
    let target = first.websocket_url();
    let selection = model_selection(REASONING_MODEL, Some("high"));
    let retained = async {
        let mut probe = initialized_probe(&target, timeout, "session-model-restart-create").await?;
        let session = create_session(
            &mut probe,
            "model-restart",
            session_config(REASONING_MODEL, Some("high")),
        )
        .await?;
        let state = session_state(&mut probe, &session.session).await?;
        assert_session_config(&state, &selection)?;
        probe.close().await?;
        Ok(session)
    }
    .await;
    let stopped = stop(first).await;
    let retained = retained?;
    stopped?;

    let restarted = stack.start_host().await?;
    let target = restarted.websocket_url();
    let verified = async {
        let mut probe = initialized_probe(&target, timeout, "session-model-restart-verify").await?;
        let state = session_state(&mut probe, &retained.session).await?;
        assert_session_config(&state, &selection)?;
        let request = stack.fixture_requests().len();
        let started = complete_turn(
            &mut probe,
            &retained.chat,
            "session-model-restart-turn",
            fixture_prompt("post restart model default", 1, 0),
            None,
        )
        .await?;
        assert_action_model(&started, &selection)?;
        let history = chat_state(&mut probe, &retained.chat).await?;
        assert_historical_model(&history, "session-model-restart-turn", &selection)?;
        assert_fixture_request(stack, request, REASONING_MODEL, "high").await?;
        probe.close().await
    }
    .await;
    let stopped = stop(restarted).await;
    verified?;
    stopped?;
    Ok("retains selected session model config through restart and applies it to a new turn".into())
}

async fn stop(host: advanced_workspace_host::RunningAdvancedHost) -> Result<(), ConformanceError> {
    host.shutdown()
        .await
        .map_err(|error| ConformanceError::Fixture(error.to_string()))
}
