//! Multi-client replay, model-failure, and shared probes.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::{InitializeResult, SubscribeResult};
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::{ConformanceError, Probe, ScenarioCheck, ScenarioRun};

mod replay;

pub(super) async fn synchronization_replay(
    target: &str,
    timeout_duration: Duration,
    exact_output: bool,
    auth_token: Option<&str>,
) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut writer = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    writer.set_auth_token(auth_token);
    let (_session, chat, _) = match ready_chat(&mut writer, "sync").await {
        Ok(ready) => ready,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = writer
        .request("subscribe", json!({ "channel": chat }))
        .await
    {
        return run.abort(error);
    }

    let mut observer = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = observer
        .initialize("sync-observer", vec![SUPPORTED_PROTOCOL_VERSIONS[0]])
        .await
    {
        return run.abort(error);
    }
    if let Err(error) = observer
        .request("subscribe", json!({ "channel": chat }))
        .await
    {
        return run.abort(error);
    }
    if let Err(error) = start_turn(
        &mut writer,
        &chat,
        "sync-live-turn",
        prompt_for_model(exact_output, "first synchronized response", 2, 0),
    )
    .await
    {
        return run.abort(error);
    }
    let live = match observer.wait_action(&chat, "chat/turnStarted").await {
        Ok(live) => live,
        Err(error) => return run.abort(error),
    };
    let last_seen = match action_sequence(&live) {
        Ok(sequence) => sequence,
        Err(error) => return run.abort(error),
    };
    run.push(ScenarioCheck::passed(
        "multi-client-live-delivery",
        "a second subscribed client observed the live turn",
    ));
    if let Err(error) = writer
        .wait_action_any(&chat, &["chat/turnComplete", "chat/error"])
        .await
    {
        return run.abort(error);
    }
    if let Err(error) = observer.close().await {
        return run.abort(error);
    }

    if let Err(error) = start_turn(
        &mut writer,
        &chat,
        "sync-replayed-turn",
        prompt_for_model(exact_output, "second replayed response", 2, 0),
    )
    .await
    {
        return run.abort(error);
    }
    if let Err(error) = writer
        .wait_action_any(&chat, &["chat/turnComplete", "chat/error"])
        .await
    {
        return run.abort(error);
    }
    record_retained_turns_snapshot(&mut run, target, timeout_duration, &chat).await;

    let mut reconnected = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let replay = match reconnected
        .request(
            "reconnect",
            json!({
                "channel": "ahp-root://",
                "clientId": "sync-observer",
                "lastSeenServerSeq": last_seen,
                "subscriptions": ["ahp-root://", chat],
            }),
        )
        .await
    {
        Ok(replay) => replay,
        Err(error) => return run.abort(error),
    };
    if replay.pointer("/result/type").and_then(Value::as_str) != Some("replay") {
        run.push(ScenarioCheck::failed(
            "reconnect-action-replay",
            "reconnect did not return an action replay",
        ));
        return run;
    }
    let actions = match replay.pointer("/result/actions").and_then(Value::as_array) {
        Some(actions) => actions,
        None => {
            run.push(ScenarioCheck::failed(
                "reconnect-action-replay",
                "reconnect omitted replay actions",
            ));
            return run;
        }
    };
    match replay::validate_sequence(actions, last_seen) {
        Ok(()) => run.push(ScenarioCheck::passed(
            "reconnect-action-replay",
            "reconnect returned actions with strictly increasing server sequences",
        )),
        Err(error) => run.push(ScenarioCheck::failed("reconnect-action-replay", error)),
    }
    match replay::validate_replayed_turn(
        actions,
        last_seen,
        &chat,
        "sync-replayed-turn",
        exact_output.then_some("second replayed response"),
    ) {
        Ok(()) => run.push(ScenarioCheck::passed(
            "replay-turn-lifecycle",
            "replay contained one complete ordered turn lifecycle",
        )),
        Err(error) => run.push(ScenarioCheck::failed("replay-turn-lifecycle", error)),
    }
    run
}

async fn record_retained_turns_snapshot(
    run: &mut ScenarioRun,
    target: &str,
    timeout_duration: Duration,
    chat: &str,
) {
    let result = async {
        let mut probe = Probe::connect(target, timeout_duration).await?;
        probe
            .initialize(
                &format!("retained-turns-{}", Uuid::new_v4()),
                vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
            )
            .await?;
        let reply = probe
            .request("subscribe", json!({ "channel": chat }))
            .await?;
        let turns = reply
            .pointer("/result/snapshot/state/turns")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                ConformanceError::Options("chat snapshot omitted retained turns".to_owned())
            })?;
        let contains = |turn_id: &str| {
            turns
                .iter()
                .any(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
        };
        probe.close().await?;
        if contains("sync-live-turn") && contains("sync-replayed-turn") {
            Ok(())
        } else {
            Err(ConformanceError::Options(
                "snapshot with omitted view did not return all retained turns".to_owned(),
            ))
        }
    }
    .await;
    match result {
        Ok(()) => run.push(ScenarioCheck::passed(
            "subscribe-retains-all-turns",
            "chat subscribe with no view returned all retained turns",
        )),
        Err(error) => run.push(ScenarioCheck::failed("subscribe-retains-all-turns", error)),
    }
}

pub(super) async fn errors(target: &str, timeout_duration: Duration) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let malformed = match probe.send_text("{not valid JSON").await {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    if malformed.pointer("/error/code").and_then(Value::as_i64) != Some(-32700) {
        run.push(ScenarioCheck::failed(
            "jsonrpc-parse-error",
            "malformed JSON did not produce Parse error",
        ));
    } else {
        run.push(ScenarioCheck::passed(
            "jsonrpc-parse-error",
            "malformed JSON returned Parse error (-32700)",
        ));
    }
    if let Err(error) = probe
        .initialize("errors-client", vec![SUPPORTED_PROTOCOL_VERSIONS[0]])
        .await
    {
        return run.abort(error);
    }
    let unknown = match probe
        .request(
            "conformance/methodThatMustNotExist",
            json!({ "channel": "ahp-root://" }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    if unknown.pointer("/error/code").and_then(Value::as_i64) != Some(-32601) {
        run.push(ScenarioCheck::failed(
            "jsonrpc-method-not-found",
            ConformanceError::Options(format!(
                "unsupported method returned {}, expected Method not found (-32601)",
                error_summary(&unknown)
            )),
        ));
    } else {
        run.push(ScenarioCheck::passed(
            "jsonrpc-method-not-found",
            "unknown method returned Method not found (-32601)",
        ));
    }
    let invalid = match probe.request("subscribe", json!({})).await {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    if invalid.pointer("/error/code").and_then(Value::as_i64) != Some(-32602) {
        run.push(ScenarioCheck::failed(
            "jsonrpc-invalid-params",
            ConformanceError::Options(format!(
                "invalid parameters returned {}, expected Invalid params (-32602)",
                error_summary(&invalid)
            )),
        ));
    } else {
        run.push(ScenarioCheck::passed(
            "jsonrpc-invalid-params",
            "invalid subscribe parameters returned Invalid params (-32602)",
        ));
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
    run
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
        .pointer("/params/action/part/error/errorType")
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
    let (provider, multiple_chats, protected_resources) = initialized
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
                        agent.protected_resources.clone().unwrap_or_default(),
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
    if let Some(token) = probe.auth_token().map(str::to_owned) {
        for resource in protected_resources {
            let reply = probe
                .request(
                    "authenticate",
                    json!({
                        "channel": "ahp-root://",
                        "resource": resource.resource,
                        "token": token.as_str(),
                    }),
                )
                .await?;
            if let Some(error) = reply.get("error") {
                return Err(ConformanceError::Options(format!(
                    "authenticate rejected an advertised protected resource: {error}"
                )));
            }
            if reply.get("result").is_none()
                || reply
                    .pointer("/result/authenticated")
                    .and_then(Value::as_bool)
                    == Some(false)
            {
                return Err(ConformanceError::Options(
                    "authenticate did not accept an advertised protected resource".to_owned(),
                ));
            }
        }
    }
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
