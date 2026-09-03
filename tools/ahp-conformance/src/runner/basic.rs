//! Protocol negotiation, snapshot, and JSON-RPC error probes.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::{InitializeResult, SubscribeResult};
use ahp_types::errors::ahp_error_codes;
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use tokio::time::{sleep, Instant};

use super::advanced::{ready_chat, start_turn};
use super::{ConformanceError, Probe, ScenarioCheck, ScenarioRun};

pub(super) async fn ping(target: &str, timeout_duration: Duration) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let before = match probe
        .request("ping", json!({ "channel": "ahp-root://" }))
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    record_null_result(
        &mut run,
        "ping-before-initialize",
        &before,
        "ping succeeded before initialize",
    );

    if let Err(error) = probe
        .initialize("ping-client", vec![SUPPORTED_PROTOCOL_VERSIONS[0]])
        .await
    {
        return run.abort(error);
    }
    let after = match probe
        .request("ping", json!({ "channel": "ahp-root://" }))
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    record_null_result(
        &mut run,
        "ping-after-initialize",
        &after,
        "ping succeeded after initialize",
    );
    run
}

fn record_null_result(
    run: &mut ScenarioRun,
    name: &'static str,
    reply: &Value,
    detail: &'static str,
) {
    if reply.get("result").is_some_and(Value::is_null) {
        run.push(ScenarioCheck::passed(name, detail));
    } else {
        run.push(ScenarioCheck::failed(
            name,
            ConformanceError::Options(format!(
                "ping did not return a null JSON-RPC result: {reply}"
            )),
        ));
    }
}

pub(super) async fn negotiation(target: &str, timeout_duration: Duration) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut supported = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let init = match supported
        .initialize(
            "negotiation-supported",
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let initialized: InitializeResult = match Probe::decode_result(&init, "initialize") {
        Ok(initialized) => initialized,
        Err(error) => return run.abort(error),
    };
    if initialized.protocol_version == SUPPORTED_PROTOCOL_VERSIONS[0] {
        run.push(ScenarioCheck::passed(
            "initialize-version",
            "host selected the offered protocol version",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "initialize-version",
            ConformanceError::Options(format!(
                "host selected {:?}, not the offered version {:?}",
                initialized.protocol_version, SUPPORTED_PROTOCOL_VERSIONS[0]
            )),
        ));
    }
    if initialized.snapshots.iter().any(|snapshot| {
        snapshot.resource == "ahp-root://" && matches!(snapshot.state, SnapshotState::Root(_))
    }) {
        run.push(ScenarioCheck::passed(
            "initialize-root-snapshot",
            "initialize returned a typed root snapshot",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "initialize-root-snapshot",
            ConformanceError::Options("initialize did not return a typed root snapshot".to_owned()),
        ));
    }
    if let Err(error) = supported.close().await {
        return run.abort(error);
    }

    let mut unsupported = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    let reply = match unsupported
        .request(
            "initialize",
            json!({
                "channel": "ahp-root://",
                "protocolVersions": ["999.0.0"],
                "clientId": "negotiation-unsupported",
            }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let code = match reply.pointer("/error/code").and_then(Value::as_i64) {
        Some(code) => code,
        None => {
            run.push(ScenarioCheck::failed(
                "initialize-unsupported-version",
                "unsupported initialize was accepted",
            ));
            return run;
        }
    };
    if code == i64::from(ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION)
        && reply.pointer("/error/data/supportedVersions").is_some()
    {
        run.push(ScenarioCheck::passed(
            "initialize-unsupported-version",
            "host rejected an incompatible offer with supportedVersions",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "initialize-unsupported-version",
            ConformanceError::Options(format!(
                "unsupported version returned code {code} with supportedVersions={}, expected code {} and supportedVersions",
                reply.pointer("/error/data/supportedVersions").is_some(),
                ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION
            )),
        ));
    }
    run
}

pub(super) async fn sessions(
    target: &str,
    timeout_duration: Duration,
    auth_token: Option<&str>,
) -> ScenarioRun {
    let mut run = ScenarioRun::new();
    let mut probe = match Probe::connect(target, timeout_duration).await {
        Ok(probe) => probe,
        Err(error) => return run.abort(error),
    };
    probe.set_auth_token(auth_token);
    let (session, chat, multiple_chats) = match ready_chat(&mut probe, "sessions").await {
        Ok(ready) => ready,
        Err(error) => return run.abort(error),
    };
    run.push(ScenarioCheck::passed(
        "session-default-chat",
        "created a session with a typed default chat",
    ));
    let reply = match probe.request("subscribe", json!({ "channel": chat })).await {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let subscribed: SubscribeResult = match Probe::decode_result(&reply, "chat subscribe") {
        Ok(subscribed) => subscribed,
        Err(error) => return run.abort(error),
    };
    match subscribed.snapshot {
        Some(snapshot)
            if snapshot.resource == chat && matches!(snapshot.state, SnapshotState::Chat(_)) =>
        {
            run.push(ScenarioCheck::passed(
                "chat-subscription-snapshot",
                "chat subscription returned the matching typed snapshot",
            ));
        }
        Some(_) => run.push(ScenarioCheck::failed(
            "chat-subscription-snapshot",
            "chat subscription returned the wrong typed snapshot",
        )),
        None => run.push(ScenarioCheck::failed(
            "chat-subscription-snapshot",
            "chat subscription omitted a snapshot",
        )),
    }
    if let Err(error) = start_turn(
        &mut probe,
        &chat,
        "session-listing-turn",
        "Reply with a short acknowledgement.".to_owned(),
    )
    .await
    {
        return run.abort(error);
    }
    let started = match probe.wait_action(&chat, "chat/turnStarted").await {
        Ok(started) => started,
        Err(error) => return run.abort(error),
    };
    if started
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
    {
        run.push(ScenarioCheck::failed(
            "session-listing",
            "session activation turn was rejected",
        ));
    } else {
        match await_listed_session(&mut probe, &session, timeout_duration).await {
            Ok(()) => run.push(ScenarioCheck::passed(
                "session-listing",
                "active session appeared in listSessions",
            )),
            Err(error) => run.push(ScenarioCheck::failed("session-listing", error)),
        }
    }
    if multiple_chats {
        let second_chat = format!("ahp-chat:/{}", uuid::Uuid::new_v4());
        let created = match probe
            .request(
                "createChat",
                json!({ "channel": session, "chat": second_chat }),
            )
            .await
        {
            Ok(created) => created,
            Err(error) => return run.abort(error),
        };
        if created.get("error").is_some() {
            run.push(ScenarioCheck::failed(
                "additional-chat",
                ConformanceError::Options(format!(
                    "agent advertised multipleChats but rejected createChat: {}",
                    created
                        .get("error")
                        .map_or_else(|| "no error object".to_owned(), Value::to_string)
                )),
            ));
            return run;
        }
        let reply = match probe
            .request("subscribe", json!({ "channel": second_chat }))
            .await
        {
            Ok(reply) => reply,
            Err(error) => return run.abort(error),
        };
        let subscribed: SubscribeResult =
            match Probe::decode_result(&reply, "second chat subscribe") {
                Ok(subscribed) => subscribed,
                Err(error) => return run.abort(error),
            };
        match subscribed.snapshot {
            Some(snapshot)
                if snapshot.resource == second_chat
                    && matches!(snapshot.state, SnapshotState::Chat(_)) =>
            {
                run.push(ScenarioCheck::passed(
                    "additional-chat",
                    "advertised multipleChats accepted and exposed another typed chat",
                ));
            }
            Some(_) => run.push(ScenarioCheck::failed(
                "additional-chat",
                "created chat subscription returned the wrong typed snapshot",
            )),
            None => run.push(ScenarioCheck::failed(
                "additional-chat",
                "created chat subscription omitted a snapshot",
            )),
        }
    } else {
        run.push(ScenarioCheck::not_applicable(
            "additional-chat",
            "agent did not advertise multipleChats",
        ));
    }
    run
}

async fn await_listed_session(
    probe: &mut Probe,
    session: &str,
    timeout_duration: Duration,
) -> Result<(), ConformanceError> {
    let deadline = Instant::now() + timeout_duration;
    loop {
        let listed = probe
            .request(
                "listSessions",
                json!({ "channel": "ahp-root://", "limit": 100 }),
            )
            .await?;
        let items = listed
            .pointer("/result/items")
            .and_then(Value::as_array)
            .ok_or_else(|| ConformanceError::Options("listSessions omitted items".to_owned()))?;
        if items
            .iter()
            .any(|item| item.get("resource").and_then(Value::as_str) == Some(session))
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let resources = items
                .iter()
                .filter_map(|item| item.get("resource").and_then(Value::as_str))
                .collect::<Vec<_>>();
            return Err(ConformanceError::Options(format!(
                "created session {session:?} did not appear in listSessions; last returned {resources:?}"
            )));
        }
        sleep(Duration::from_millis(100)).await;
    }
}
