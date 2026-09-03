//! Protocol negotiation, snapshot, and JSON-RPC error probes.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::{InitializeResult, SubscribeResult};
use ahp_types::errors::ahp_error_codes;
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use base64::Engine;
use serde_json::{json, Value};
use tokio::time::{sleep, Instant};

use super::advanced::{ready_chat, start_turn};
use super::{ConformanceError, Probe};

pub(super) async fn negotiation(
    target: &str,
    timeout_duration: Duration,
) -> Result<String, ConformanceError> {
    let mut supported = Probe::connect(target, timeout_duration).await?;
    let init = supported
        .initialize(
            "negotiation-supported",
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    let initialized: InitializeResult = Probe::decode_result(&init, "initialize")?;
    if initialized.protocol_version != SUPPORTED_PROTOCOL_VERSIONS[0] {
        return Err(ConformanceError::Options(format!(
            "host selected {:?}, not an offered version",
            initialized.protocol_version
        )));
    }
    if !initialized.snapshots.iter().any(|snapshot| {
        snapshot.resource == "ahp-root://" && matches!(snapshot.state, SnapshotState::Root(_))
    }) {
        return Err(ConformanceError::Options(
            "initialize did not return a typed root snapshot".to_owned(),
        ));
    }
    supported.close().await?;

    let mut unsupported = Probe::connect(target, timeout_duration).await?;
    let reply = unsupported
        .request(
            "initialize",
            json!({
                "channel": "ahp-root://",
                "protocolVersions": ["999.0.0"],
                "clientId": "negotiation-unsupported",
            }),
        )
        .await?;
    let code = reply
        .pointer("/error/code")
        .and_then(Value::as_i64)
        .ok_or_else(|| {
            ConformanceError::Options("unsupported initialize was accepted".to_owned())
        })?;
    if code != i64::from(ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION) {
        return Err(ConformanceError::Options(format!(
            "unsupported version returned {code}, expected {}",
            ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION
        )));
    }
    if reply.pointer("/error/data/supportedVersions").is_none() {
        return Err(ConformanceError::Options(
            "unsupported-version error omitted supportedVersions".to_owned(),
        ));
    }
    Ok("negotiates an offered version and rejects an incompatible offer".to_owned())
}

pub(super) async fn sessions(
    target: &str,
    timeout_duration: Duration,
) -> Result<String, ConformanceError> {
    let mut probe = Probe::connect(target, timeout_duration).await?;
    let (session, chat, multiple_chats) = ready_chat(&mut probe, "sessions").await?;
    let reply = probe
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    let subscribed: SubscribeResult = Probe::decode_result(&reply, "chat subscribe")?;
    let snapshot = subscribed.snapshot.ok_or_else(|| {
        ConformanceError::Options("chat subscription omitted a snapshot".to_owned())
    })?;
    if snapshot.resource != chat || !matches!(snapshot.state, SnapshotState::Chat(_)) {
        return Err(ConformanceError::Options(
            "chat subscription returned the wrong typed snapshot".to_owned(),
        ));
    }
    start_turn(
        &mut probe,
        &chat,
        "session-listing-turn",
        "Reply with a short acknowledgement.".to_owned(),
    )
    .await?;
    let started = probe.wait_action(&chat, "chat/turnStarted").await?;
    if started
        .pointer("/params/rejectionReason")
        .and_then(Value::as_str)
        .is_some_and(|reason| !reason.is_empty())
    {
        return Err(ConformanceError::Options(
            "session activation turn was rejected".to_owned(),
        ));
    }
    await_listed_session(&mut probe, &session, timeout_duration).await?;

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
                .ok_or_else(|| {
                    ConformanceError::Options("listSessions omitted items".to_owned())
                })?;
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
    if multiple_chats {
        let canonical_session = decode_chat_session(&chat).unwrap_or_else(|| session.clone());
        let second_chat = build_chat_uri(&canonical_session, &uuid::Uuid::new_v4().to_string());
        let created = probe
            .request(
                "createChat",
                json!({ "channel": session, "chat": second_chat }),
            )
            .await?;
        if created.get("error").is_some() {
            return Err(ConformanceError::Options(format!(
                "agent advertised multipleChats but rejected createChat: {}",
                created
                    .get("error")
                    .map_or_else(|| "no error object".to_owned(), Value::to_string)
            )));
        }

        fn build_chat_uri(session: &str, chat_id: &str) -> String {
            let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(session);
            format!("ahp-chat://{chat_id}/{encoded}")
        }

        fn decode_chat_session(chat: &str) -> Option<String> {
            let url = url::Url::parse(chat).ok()?;
            let encoded = url.path().trim_start_matches('/');
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(encoded)
                .ok()?;
            String::from_utf8(bytes).ok()
        }
        let reply = probe
            .request("subscribe", json!({ "channel": second_chat }))
            .await?;
        let subscribed: SubscribeResult = Probe::decode_result(&reply, "second chat subscribe")?;
        let snapshot = subscribed.snapshot.ok_or_else(|| {
            ConformanceError::Options("created chat subscription omitted a snapshot".to_owned())
        })?;
        if snapshot.resource != second_chat || !matches!(snapshot.state, SnapshotState::Chat(_)) {
            return Err(ConformanceError::Options(
                "created chat subscription returned the wrong typed snapshot".to_owned(),
            ));
        }
    }
    Ok(if multiple_chats {
        "creates, lists, and subscribes to typed session/chat snapshots and an advertised second chat"
    } else {
        "creates, activates, lists, and subscribes to typed session/chat snapshots; multiple chats were not advertised"
    }
    .to_owned())
}
