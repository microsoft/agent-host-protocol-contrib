//! Multi-client replay, model-failure, and shared probes.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::{InitializeResult, SubscribeResult};
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ConformanceError, Probe};

mod replay;

pub(super) async fn synchronization_replay(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
) -> Result<String, ConformanceError> {
    let mut writer = Probe::connect(target, timeout_duration).await?;
    let (_session, chat, _) = ready_chat(&mut writer, "sync").await?;
    writer
        .request("subscribe", json!({ "channel": chat }))
        .await?;

    let mut observer = Probe::connect(target, timeout_duration).await?;
    observer
        .initialize("sync-observer", vec![SUPPORTED_PROTOCOL_VERSIONS[0]])
        .await?;
    observer
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    start_turn(
        &mut writer,
        &chat,
        "sync-live-turn",
        prompt_for_model(exact_output, "first synchronized response", 2, 0),
    )
    .await?;
    let live = observer.wait_action(&chat, "chat/turnStarted").await?;
    let last_seen = action_sequence(&live)?;
    writer
        .wait_action_any(&chat, &["chat/turnComplete", "chat/error"])
        .await?;
    observer.close().await?;

    start_turn(
        &mut writer,
        &chat,
        "sync-replayed-turn",
        prompt_for_model(exact_output, "second replayed response", 2, 0),
    )
    .await?;
    writer
        .wait_action_any(&chat, &["chat/turnComplete", "chat/error"])
        .await?;

    let mut reconnected = Probe::connect(target, timeout_duration).await?;
    let replay = reconnected
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": "sync-observer",
                "lastSeenServerSeq": last_seen,
                "subscriptions": ["ahp-root://", chat],
            }),
        )
        .await?;
    if replay.pointer("/result/type").and_then(Value::as_str) != Some("replay") {
        return Err(ConformanceError::Options(
            "reconnect did not return an action replay".to_owned(),
        ));
    }
    let actions = replay
        .pointer("/result/actions")
        .and_then(Value::as_array)
        .ok_or_else(|| ConformanceError::Options("reconnect omitted replay actions".to_owned()))?;
    replay::validate_replayed_turn(
        actions,
        last_seen,
        &chat,
        "sync-replayed-turn",
        exact_output.then_some("second replayed response"),
    )?;
    Ok("broadcasts to a second client and replays a complete ordered turn lifecycle".to_owned())
}

pub(super) async fn errors(
    target: &str,
    timeout_duration: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout_duration).await?;
    let malformed = probe.send_text("{not valid JSON").await?;
    if malformed.pointer("/error/code").and_then(Value::as_i64) != Some(-32700) {
        return Err(ConformanceError::Options(
            "malformed JSON did not produce Parse error".to_owned(),
        ));
    }
    probe
        .initialize("errors-client", vec![SUPPORTED_PROTOCOL_VERSIONS[0]])
        .await?;
    let unknown = probe
        .request(
            "conformance/methodThatMustNotExist",
            json!({ "channel": "ahp-root://" }),
        )
        .await?;
    if unknown.pointer("/error/code").and_then(Value::as_i64) != Some(-32601) {
        return Err(ConformanceError::Options(format!(
            "unsupported method returned {}, expected Method not found (-32601)",
            error_summary(&unknown)
        )));
    }
    let invalid = probe.request("subscribe", json!({})).await?;
    if invalid.pointer("/error/code").and_then(Value::as_i64) != Some(-32602) {
        return Err(ConformanceError::Options(format!(
            "invalid parameters returned {}, expected Invalid params (-32602)",
            error_summary(&invalid)
        )));
    }

    fn error_summary(reply: &Value) -> String {
        let Some(error) = reply.get("error") else {
            return "no error object".to_owned();
        };
        let code = error.get("code").map_or(Value::Null, Clone::clone);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .and_then(|message| message.lines().next())
            .unwrap_or("missing error message");
        json!({ "code": code, "message": message }).to_string()
    }
    Ok("returns JSON-RPC parse, method-not-found, and invalid-params errors".to_owned())
}

pub(super) async fn upstream_error(
    target: &str,
    timeout_duration: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout_duration).await?;
    let (_session, chat, _) = ready_chat(&mut probe, "upstream-error").await?;
    probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    start_turn(
        &mut probe,
        &chat,
        "upstream-error-turn",
        fixture_upstream_error_prompt(),
    )
    .await?;
    let error = probe.wait_action(&chat, "chat/error").await?;
    if error
        .pointer("/params/action/error/errorType")
        .and_then(Value::as_str)
        != Some("openai_upstream_error")
    {
        return Err(ConformanceError::Options(
            "upstream failure was not surfaced as chat/error".to_owned(),
        ));
    }
    Ok("surfaces explicit OpenAI fixture failures as chat/error".to_owned())
}

pub(super) async fn ready_chat(
    probe: &mut Probe,
    suffix: &str,
) -> Result<(String, String, bool), ConformanceError> {
    let initialized = probe
        .initialize(
            &format!("{suffix}-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let initialized: InitializeResult = Probe::decode_result(&initialized, "initialize")?;
    let (provider, multiple_chats) = initialized
        .snapshots
        .iter()
        .find_map(|snapshot| match &snapshot.state {
            SnapshotState::Root(root) if snapshot.resource == "ahp-root://" => {
                root.agents.first().map(|agent| {
                    (
                        agent.provider.clone(),
                        agent
                            .capabilities
                            .as_ref()
                            .and_then(|capabilities| capabilities.multiple_chats.as_ref())
                            .is_some(),
                    )
                })
            }
            _ => None,
        })
        .ok_or_else(|| {
            ConformanceError::Options(
                "initialize root snapshot did not advertise an agent provider".to_owned(),
            )
        })?;
    let session = format!("ahp-session:/{}", Uuid::new_v4());
    probe
        .request(
            "createSession",
            json!({ "channel": session, "provider": provider }),
        )
        .await?;
    let reply = probe
        .request("subscribe", json!({ "channel": session }))
        .await?;
    let subscribed: SubscribeResult = Probe::decode_result(&reply, "session subscribe")?;
    let snapshot = subscribed.snapshot.ok_or_else(|| {
        ConformanceError::Options("session subscription omitted a snapshot".to_owned())
    })?;
    if snapshot.resource != session {
        return Err(ConformanceError::Options(
            "session subscription returned the wrong resource".to_owned(),
        ));
    }
    let SnapshotState::Session(session_state) = snapshot.state else {
        return Err(ConformanceError::Options(
            "session subscription did not return SessionState".to_owned(),
        ));
    };
    let chat = session_state
        .default_chat
        .ok_or_else(|| ConformanceError::Options("session has no default chat".to_owned()))?;
    Ok((session, chat, multiple_chats))
}

pub(super) async fn start_turn(
    probe: &mut Probe,
    chat: &str,
    turn_id: &str,
    prompt: String,
) -> Result<(), ConformanceError> {
    probe
        .dispatch_action(
            chat,
            json!({
                "type": "chat/turnStarted",
                "turnId": turn_id,
                "startedAt": "2026-01-02T03:04:05.000Z",
                "message": { "text": prompt, "origin": { "kind": "user" } },
            }),
        )
        .await
}

pub(super) fn fixture_prompt(response: &str, chunks: usize, delay_ms: u64) -> String {
    let response = Value::String(response.to_owned()).to_string();
    format!(
        "For deterministic test behavior, please follow this instruction:\n[AHP-TEST v1]\nRespond exactly with JSON: {response}\nStream in exactly {chunks} chunks.\nWait {delay_ms} ms before every output chunk.\n[/AHP-TEST]"
    )
}

pub(super) fn prompt_for_model(
    exact_output: bool,
    expected: &str,
    chunks: usize,
    delay_ms: u64,
) -> String {
    if exact_output {
        fixture_prompt(expected, chunks, delay_ms)
    } else {
        format!("Please provide a brief response about: {expected}.")
    }
}

fn fixture_upstream_error_prompt() -> String {
    "[AHP-TEST v1]\nReturn upstream HTTP 503 with JSON message: \"planned fixture outage\".\n[/AHP-TEST]"
        .to_owned()
}

pub(super) fn action_sequence(action: &Value) -> Result<i64, ConformanceError> {
    action
        .pointer("/params/serverSeq")
        .and_then(Value::as_i64)
        .ok_or_else(|| ConformanceError::Options("action omitted serverSeq".to_owned()))
}
