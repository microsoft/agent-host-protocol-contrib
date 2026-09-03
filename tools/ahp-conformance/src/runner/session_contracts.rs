//! Session catalogue, lifecycle, pagination, and subscription contracts.
//!
//! Internal conformance-runner module.

use std::time::Duration;

use ahp_types::commands::{InitializeResult, ListSessionsResult};
use ahp_types::errors::{ahp_error_codes, json_rpc_error_codes};
use ahp_types::state::SnapshotState;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

use super::advanced::ready_chat;
use super::{ConformanceError, Probe, ScenarioCheck, ScenarioRun};

const UNSUBSCRIBE_QUIESCENCE: Duration = Duration::from_millis(500);

pub(super) async fn run(
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
    let (session, chat, _) = match ready_chat(&mut probe, "session-contracts").await {
        Ok(ready) => ready,
        Err(error) => return run.abort(error),
    };

    match probe
        .wait_notification_for_resource("root/sessionAdded", "ahp-root://", &session)
        .await
    {
        Ok(_) => run.push(ScenarioCheck::passed(
            "root-session-added",
            "session creation broadcast root/sessionAdded to a root subscriber",
        )),
        Err(error) => run.push(ScenarioCheck::failed("root-session-added", error)),
    }

    let listed = match probe
        .request(
            "listSessions",
            json!({
                "channel": "ahp-root://",
                "_meta": { "com.example.future": { "nested": true } }
            }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    if let Err(error) = result_items(&listed) {
        return run.abort(error);
    }
    run.push(ScenarioCheck::passed(
        "unknown-metadata-ignored",
        "listSessions ignored an unknown namespaced _meta value",
    ));
    match Probe::decode_result::<ListSessionsResult>(&listed, "listSessions") {
        Ok(_) => run.push(ScenarioCheck::passed(
            "session-summary-shape",
            "listSessions returned typed session summaries",
        )),
        Err(error) => run.push(ScenarioCheck::failed("session-summary-shape", error)),
    }
    let provider_reply = match probe
        .request("subscribe", json!({ "channel": session }))
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    let provider = match provider_reply
        .pointer("/result/snapshot/state/provider")
        .and_then(Value::as_str)
    {
        Some(provider) => provider.to_owned(),
        None => return run.abort("session snapshot omitted its provider"),
    };

    record_initial_subscription_snapshots(&mut run, target, timeout_duration, &session, &chat)
        .await;

    let duplicate = match probe
        .request(
            "createSession",
            json!({ "channel": session, "provider": provider }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    record_error_code(
        &mut run,
        "session-duplicate-error",
        &duplicate,
        i64::from(ahp_error_codes::SESSION_ALREADY_EXISTS),
        "duplicate session URI returned SessionAlreadyExists",
    );

    let mut auxiliaries = Vec::new();
    for _ in 0..2 {
        sleep(Duration::from_millis(10)).await;
        let auxiliary = format!("ahp-session:/{}", Uuid::new_v4());
        if let Err(error) = create_session(&mut probe, &auxiliary, &provider).await {
            return run.abort(error);
        }
        auxiliaries.push(auxiliary);
    }

    record_pagination_contracts(&mut run, &mut probe).await;

    let fetch = match probe
        .request(
            "fetchTurns",
            json!({ "channel": chat, "cursor": "conformance-unknown-cursor" }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    record_error_code(
        &mut run,
        "fetch-turns-invalid-cursor",
        &fetch,
        i64::from(json_rpc_error_codes::INVALID_PARAMS),
        "fetchTurns rejected an unrecognised cursor with Invalid params",
    );

    match verify_unsubscribe(target, timeout_duration, &mut probe, &chat).await {
        Ok(()) => run.push(ScenarioCheck::passed(
            "unsubscribe-stops-delivery",
            "a previously live chat subscription stopped receiving actions after unsubscribe",
        )),
        Err(error) => run.push(ScenarioCheck::failed("unsubscribe-stops-delivery", error)),
    }

    for auxiliary in &auxiliaries {
        if let Err(error) = dispose_session(&mut probe, auxiliary).await {
            return run.abort(error);
        }
    }
    if let Err(error) = dispose_session(&mut probe, &session).await {
        return run.abort(error);
    }
    match probe
        .wait_notification_for_resource("root/sessionRemoved", "ahp-root://", &session)
        .await
    {
        Ok(_) => run.push(ScenarioCheck::passed(
            "root-session-removed",
            "session disposal broadcast root/sessionRemoved to a root subscriber",
        )),
        Err(error) => run.push(ScenarioCheck::failed("root-session-removed", error)),
    }
    let listed = match probe
        .request("listSessions", json!({ "channel": "ahp-root://" }))
        .await
    {
        Ok(reply) => reply,
        Err(error) => return run.abort(error),
    };
    match result_items(&listed) {
        Ok(items)
            if items
                .iter()
                .all(|item| item.get("resource").and_then(Value::as_str) != Some(&session)) =>
        {
            run.push(ScenarioCheck::passed(
                "disposed-session-absent",
                "disposed session was removed from listSessions",
            ));
        }
        Ok(_) => run.push(ScenarioCheck::failed(
            "disposed-session-absent",
            "disposed session remained in listSessions",
        )),
        Err(error) => return run.abort(error),
    }

    run
}

async fn verify_unsubscribe(
    target: &str,
    timeout_duration: Duration,
    writer: &mut Probe,
    chat: &str,
) -> Result<(), ConformanceError> {
    writer
        .request("subscribe", json!({ "channel": chat }))
        .await?;
    let mut observer = Probe::connect(target, timeout_duration).await?;
    observer
        .initialize(
            &format!("unsubscribe-observer-{}", Uuid::new_v4()),
            vec![SUPPORTED_PROTOCOL_VERSIONS[0]],
        )
        .await?;
    observer
        .request("subscribe", json!({ "channel": chat }))
        .await?;

    let before_seq = writer
        .dispatch_action_with_sequence(
            chat,
            json!({
                "type": "chat/draftChanged",
                "draft": {
                    "text": "before unsubscribe",
                    "origin": { "kind": "user" }
                }
            }),
        )
        .await?;
    let live = observer.wait_action(chat, "chat/draftChanged").await?;
    require_origin_sequence(&live, before_seq)?;
    let echoed = writer.wait_action(chat, "chat/draftChanged").await?;
    require_origin_sequence(&echoed, before_seq)?;

    observer
        .notify("unsubscribe", json!({ "channel": chat }))
        .await?;
    let after_seq = writer
        .dispatch_action_with_sequence(
            chat,
            json!({
                "type": "chat/draftChanged",
                "draft": {
                    "text": "after unsubscribe",
                    "origin": { "kind": "user" }
                }
            }),
        )
        .await?;
    let echoed = writer.wait_action(chat, "chat/draftChanged").await?;
    require_origin_sequence(&echoed, after_seq)?;
    let quiet = observer
        .assert_no_action_for_client_sequence(
            chat,
            "chat/draftChanged",
            after_seq,
            UNSUBSCRIBE_QUIESCENCE,
        )
        .await;
    observer.close().await?;
    quiet
}

fn require_origin_sequence(action: &Value, expected: i64) -> Result<(), ConformanceError> {
    let actual = action
        .pointer("/params/origin/clientSeq")
        .and_then(Value::as_i64);
    if actual == Some(expected) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "authoritative action origin was {actual:?}, expected clientSeq {expected}"
        )))
    }
}

async fn record_initial_subscription_snapshots(
    run: &mut ScenarioRun,
    target: &str,
    timeout_duration: Duration,
    session: &str,
    chat: &str,
) {
    let result = async {
        let mut observer = Probe::connect(target, timeout_duration).await?;
        let reply = observer
            .request(
                "initialize",
                json!({
                    "channel": "ahp-root://",
                    "protocolVersions": [SUPPORTED_PROTOCOL_VERSIONS[0]],
                    "clientId": format!("initial-subscriptions-{}", Uuid::new_v4()),
                    "initialSubscriptions": ["ahp-root://", session, chat],
                }),
            )
            .await?;
        let initialized: InitializeResult =
            Probe::decode_result(&reply, "initialize with three subscriptions")?;
        let root = initialized.snapshots.iter().any(|snapshot| {
            snapshot.resource == "ahp-root://" && matches!(snapshot.state, SnapshotState::Root(_))
        });
        let session_snapshot = initialized.snapshots.iter().any(|snapshot| {
            snapshot.resource == session && matches!(snapshot.state, SnapshotState::Session(_))
        });
        let chat_snapshot = initialized.snapshots.iter().any(|snapshot| {
            snapshot.resource == chat && matches!(snapshot.state, SnapshotState::Chat(_))
        });
        observer.close().await?;
        if root && session_snapshot && chat_snapshot {
            Ok(())
        } else {
            Err(ConformanceError::Options(
                "initialize did not return typed snapshots for every initial subscription"
                    .to_owned(),
            ))
        }
    }
    .await;
    match result {
        Ok(()) => run.push(ScenarioCheck::passed(
            "initial-subscription-snapshots",
            "initialize returned typed snapshots for all initial subscriptions",
        )),
        Err(error) => run.push(ScenarioCheck::failed(
            "initial-subscription-snapshots",
            error,
        )),
    }
}

async fn record_pagination_contracts(run: &mut ScenarioRun, probe: &mut Probe) {
    let first = match probe
        .request(
            "listSessions",
            json!({ "channel": "ahp-root://", "limit": 1 }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            record_pagination_abort(run, error);
            return;
        }
    };
    let first_items = match result_items(&first) {
        Ok(items) => items,
        Err(error) => {
            record_pagination_abort(run, error);
            return;
        }
    };
    if first_items.len() == 1 {
        run.push(ScenarioCheck::passed(
            "session-pagination-limit",
            "listSessions respected a one-item page limit",
        ));
    } else {
        run.push(ScenarioCheck::failed(
            "session-pagination-limit",
            format!(
                "listSessions returned {} items for limit 1",
                first_items.len()
            ),
        ));
    }
    let next_cursor = first.pointer("/result/nextCursor").and_then(Value::as_str);
    match next_cursor {
        Some(cursor) => {
            let second = match probe
                .request(
                    "listSessions",
                    json!({ "channel": "ahp-root://", "limit": 1, "cursor": cursor }),
                )
                .await
            {
                Ok(reply) => reply,
                Err(error) => {
                    run.push(ScenarioCheck::failed("session-pagination-cursor", error));
                    return;
                }
            };
            match result_items(&second) {
                Ok(items)
                    if items.len() == 1
                        && items[0].get("resource").and_then(Value::as_str)
                            != first_items[0].get("resource").and_then(Value::as_str) =>
                {
                    run.push(ScenarioCheck::passed(
                        "session-pagination-cursor",
                        "nextCursor returned the next distinct session page",
                    ));
                }
                Ok(_) => run.push(ScenarioCheck::failed(
                    "session-pagination-cursor",
                    "nextCursor did not return one distinct next-page item",
                )),
                Err(error) => {
                    run.push(ScenarioCheck::failed("session-pagination-cursor", error));
                }
            }
        }
        None => run.push(ScenarioCheck::failed(
            "session-pagination-cursor",
            "listSessions omitted nextCursor despite additional sessions",
        )),
    }

    let invalid = match probe
        .request(
            "listSessions",
            json!({
                "channel": "ahp-root://",
                "cursor": "conformance-unknown-cursor"
            }),
        )
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            run.push(ScenarioCheck::failed(
                "session-pagination-invalid-cursor",
                error,
            ));
            return;
        }
    };
    record_error_code(
        run,
        "session-pagination-invalid-cursor",
        &invalid,
        i64::from(json_rpc_error_codes::INVALID_PARAMS),
        "listSessions rejected an unrecognised cursor with Invalid params",
    );

    let all = match probe
        .request("listSessions", json!({ "channel": "ahp-root://" }))
        .await
    {
        Ok(reply) => reply,
        Err(error) => {
            run.push(ScenarioCheck::failed("session-list-order", error));
            return;
        }
    };
    match result_items(&all) {
        Ok(items) if modified_times_descend(items) => run.push(ScenarioCheck::passed(
            "session-list-order",
            "listSessions returned most-recently-modified sessions first",
        )),
        Ok(_) => run.push(ScenarioCheck::failed(
            "session-list-order",
            "listSessions was not ordered by descending modifiedAt",
        )),
        Err(error) => run.push(ScenarioCheck::failed("session-list-order", error)),
    }
}

fn record_pagination_abort(run: &mut ScenarioRun, error: ConformanceError) {
    let detail = error.to_string();
    for name in [
        "session-pagination-limit",
        "session-pagination-cursor",
        "session-pagination-invalid-cursor",
        "session-list-order",
    ] {
        run.push(ScenarioCheck::failed(name, &detail));
    }
}

fn result_items(reply: &Value) -> Result<&[Value], ConformanceError> {
    reply
        .pointer("/result/items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| ConformanceError::Options("listSessions omitted result.items".to_owned()))
}

fn modified_times_descend(items: &[Value]) -> bool {
    let times = items
        .iter()
        .map(|item| item.get("modifiedAt").and_then(Value::as_str))
        .collect::<Option<Vec<_>>>();
    times.is_some_and(|times| times.windows(2).all(|pair| pair[0] >= pair[1]))
}

fn record_error_code(
    run: &mut ScenarioRun,
    name: &'static str,
    reply: &Value,
    expected: i64,
    detail: &'static str,
) {
    let actual = reply.pointer("/error/code").and_then(Value::as_i64);
    if actual == Some(expected) {
        run.push(ScenarioCheck::passed(name, detail));
    } else {
        run.push(ScenarioCheck::failed(
            name,
            format!("expected JSON-RPC error code {expected}, received {actual:?}"),
        ));
    }
}

async fn create_session(
    probe: &mut Probe,
    session: &str,
    provider: &str,
) -> Result<(), ConformanceError> {
    let reply = probe
        .request(
            "createSession",
            json!({ "channel": session, "provider": provider }),
        )
        .await?;
    if reply.get("result").is_some_and(Value::is_null) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "createSession failed during lifecycle setup: {reply}"
        )))
    }
}

async fn dispose_session(probe: &mut Probe, session: &str) -> Result<(), ConformanceError> {
    let reply = probe
        .request("disposeSession", json!({ "channel": session }))
        .await?;
    if reply.get("result").is_some_and(Value::is_null) {
        Ok(())
    } else {
        Err(ConformanceError::Options(format!(
            "disposeSession failed during lifecycle cleanup: {reply}"
        )))
    }
}
