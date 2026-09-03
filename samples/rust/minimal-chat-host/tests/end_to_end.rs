#[path = "../src/host.rs"]
mod host;
#[path = "../src/protocol.rs"]
mod protocol;
#[path = "../src/responses.rs"]
mod responses;

use std::{convert::Infallible, error::Error, io, net::SocketAddr, time::Duration};

use axum::{
    body::{Body, Bytes},
    extract::Json,
    http::{header::CONTENT_TYPE, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use host::{start, HostConfig, RunningHost, CHAT_URI, SESSION_URI};
use responses::{parse_responses_event, ResponsesEvent, SseDecoder, SseEvent};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::oneshot,
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[test]
fn builds_responses_url_only_from_http_api_origins() -> TestResult {
    assert_eq!(
        responses::endpoint("https://api.example.test")?.as_str(),
        "https://api.example.test/v1/responses"
    );
    for base_url in [
        "file:///workspace",
        "mailto:agent@example.test",
        "https://api.example.test/v1",
    ] {
        assert!(responses::endpoint(base_url).is_err(), "{base_url}");
    }
    Ok(())
}

struct Fixture {
    base_url: String,
    shutdown: oneshot::Sender<()>,
    task: JoinHandle<()>,
}

impl Fixture {
    async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.task.await;
    }
}

#[test]
fn parses_fragmented_sse_lines_and_responses_events() -> TestResult {
    let mut decoder = SseDecoder::default();
    assert!(decoder
        .push(b": comment\r\nevent: response.output_")?
        .is_empty());
    let events = decoder.push(
        b"text.delta\r\ndata: {\"type\":\"response.output_text.delta\",\r\ndata: \"delta\":\"hello\"}\r\n\r\n",
    )?;

    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].event.as_deref(),
        Some("response.output_text.delta")
    );
    assert_eq!(
        parse_responses_event(&events[0])?,
        ResponsesEvent::TextDelta("hello".to_owned())
    );
    assert_eq!(
        parse_responses_event(&SseEvent {
            event: Some("response.created".to_owned()),
            data: r#"{"type":"response.created","response":{"id":"resp-1"}}"#.to_owned(),
        })?,
        ResponsesEvent::Started("resp-1".to_owned())
    );
    assert_eq!(
        parse_responses_event(&SseEvent {
            event: Some("response.completed".to_owned()),
            data: r#"{"type":"response.completed"}"#.to_owned(),
        })?,
        ResponsesEvent::Completed
    );
    assert_eq!(
        parse_responses_event(&SseEvent {
            event: Some("response.incomplete".to_owned()),
            data: r#"{"type":"response.incomplete","response":{"error":{"message":"limit"}}}"#
                .to_owned(),
        })?,
        ResponsesEvent::Failed("limit".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn unsupported_protocol_version_reports_supported_versions() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let (mut socket, _) = connect_async(host.websocket_url()).await?;

    let response = request(
        &mut socket,
        1,
        "initialize",
        json!({
            "channel": "ahp-root://",
            "clientId": "unsupported-version-test",
            "protocolVersions": ["999.0.0"],
        }),
    )
    .await?;
    assert_eq!(
        response,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32005,
                "message": "no mutually supported AHP protocol version",
                "data": { "supportedVersions": ahp_types::SUPPORTED_PROTOCOL_VERSIONS },
            },
        })
    );

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn forwards_a_delta_before_the_completed_response() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let mut socket = connect_and_initialize(&host).await?;

    start_turn(&mut socket, "finish", "turn-finish").await?;
    let mut saw_started = false;
    let mut saw_part = false;
    let mut saw_delta = false;
    loop {
        let action = next_action(&mut socket).await?;
        match action.pointer("/action/type").and_then(Value::as_str) {
            Some("chat/turnStarted") => saw_started = true,
            Some("chat/responsePart") => saw_part = true,
            Some("chat/delta") => {
                assert!(saw_started && saw_part);
                saw_delta = true;
            }
            Some("chat/turnComplete") => {
                assert!(saw_delta, "the completed action arrived before any delta");
                break;
            }
            other => return Err(io::Error::other(format!("unexpected action {other:?}")).into()),
        }
    }

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn chains_production_responses_between_turns() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host_with_chaining(&fixture).await?;
    let mut socket = connect_and_initialize(&host).await?;

    start_turn(&mut socket, "first", "chain-first").await?;
    wait_for_turn_complete(&mut socket, "chain-first").await?;
    start_turn(&mut socket, "chain-second", "chain-second").await?;
    wait_for_turn_complete(&mut socket, "chain-second").await?;

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn clears_the_response_chain_after_cancellation() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host_with_chaining(&fixture).await?;
    let mut socket = connect_and_initialize(&host).await?;

    start_turn(&mut socket, "first", "before-cancel").await?;
    wait_for_turn_complete(&mut socket, "before-cancel").await?;
    start_turn(&mut socket, "cancel-chain", "cancel-chain").await?;
    loop {
        let action = next_action(&mut socket).await?;
        if action.pointer("/action/type").and_then(Value::as_str) == Some("chat/delta")
            && action.pointer("/action/turnId").and_then(Value::as_str) == Some("cancel-chain")
        {
            break;
        }
    }
    send_action(
        &mut socket,
        2,
        json!({ "type": "chat/turnCancelled", "turnId": "cancel-chain" }),
    )
    .await?;
    wait_for_action(&mut socket, "chat/turnCancelled", 0).await?;

    start_turn(&mut socket, "after-cancel", "after-cancel").await?;
    wait_for_turn_complete(&mut socket, "after-cancel").await?;

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn subscription_response_precedes_concurrent_action_delivery() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let (mut subscriber, _) = connect_async(host.websocket_url()).await?;
    initialize(&mut subscriber, "subscriber", json!(["ahp-root://"])).await?;
    let (mut actor, _) = connect_async(host.websocket_url()).await?;
    initialize(&mut actor, "actor", json!(["ahp-root://"])).await?;

    subscriber
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "subscribe",
                "params": { "channel": CHAT_URI },
            })
            .to_string()
            .into(),
        ))
        .await?;
    send_action(
        &mut actor,
        1,
        json!({
            "type": "chat/turnStarted",
            "turnId": "concurrent-turn",
            "startedAt": "2026-08-31T00:00:00.000Z",
            "message": { "text": "cancel", "origin": { "kind": "user" } },
        }),
    )
    .await?;

    let Message::Text(first) = next_frame(&mut subscriber).await? else {
        return Err(io::Error::other("subscribe response was not text").into());
    };
    let response: Value = serde_json::from_str(&first)?;
    assert_eq!(response.get("id"), Some(&Value::from(2)));
    let from_seq = response
        .pointer("/result/snapshot/fromSeq")
        .and_then(Value::as_i64)
        .ok_or_else(|| io::Error::other("subscribe snapshot had no fromSeq"))?;
    let snapshot_has_turn = response
        .pointer("/result/snapshot/state/activeTurn/id")
        .and_then(Value::as_str)
        == Some("concurrent-turn");
    if !snapshot_has_turn {
        wait_for_action(&mut subscriber, "chat/turnStarted", from_seq).await?;
    }

    send_action(
        &mut actor,
        2,
        json!({ "type": "chat/turnCancelled", "turnId": "concurrent-turn" }),
    )
    .await?;
    wait_for_action(&mut subscriber, "chat/turnCancelled", from_seq).await?;

    subscriber.close(None).await?;
    actor.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn initialization_response_precedes_concurrent_action_delivery() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let (mut actor, _) = connect_async(host.websocket_url()).await?;
    initialize(&mut actor, "actor", json!(["ahp-root://"])).await?;
    let (mut subscriber, _) = connect_async(host.websocket_url()).await?;

    subscriber
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "channel": "ahp-root://",
                    "clientId": "initializer",
                    "protocolVersions": ["0.9.0"],
                    "initialSubscriptions": [CHAT_URI],
                },
            })
            .to_string()
            .into(),
        ))
        .await?;
    send_action(
        &mut actor,
        1,
        json!({
            "type": "chat/turnStarted",
            "turnId": "initialization-turn",
            "startedAt": "2026-08-31T00:00:00.000Z",
            "message": { "text": "cancel", "origin": { "kind": "user" } },
        }),
    )
    .await?;

    let Message::Text(first) = next_frame(&mut subscriber).await? else {
        return Err(io::Error::other("initialize response was not text").into());
    };
    let response: Value = serde_json::from_str(&first)?;
    assert_eq!(response.get("id"), Some(&Value::from(1)));
    let from_seq = response
        .pointer("/result/snapshots/0/fromSeq")
        .and_then(Value::as_i64)
        .ok_or_else(|| io::Error::other("initialize snapshot had no fromSeq"))?;
    let snapshot_has_turn = response
        .pointer("/result/snapshots/0/state/activeTurn/id")
        .and_then(Value::as_str)
        == Some("initialization-turn");
    if !snapshot_has_turn {
        wait_for_action(&mut subscriber, "chat/turnStarted", from_seq).await?;
    }
    send_action(
        &mut actor,
        2,
        json!({ "type": "chat/turnCancelled", "turnId": "initialization-turn" }),
    )
    .await?;
    wait_for_action(&mut subscriber, "chat/turnCancelled", from_seq).await?;

    subscriber.close(None).await?;
    actor.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn cancellation_drops_the_stream_and_suppresses_later_deltas() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let mut socket = connect_and_initialize(&host).await?;

    start_turn(&mut socket, "cancel", "turn-cancel").await?;
    loop {
        let action = next_action(&mut socket).await?;
        if action.pointer("/action/type").and_then(Value::as_str) == Some("chat/delta") {
            assert_eq!(
                action.pointer("/action/content").and_then(Value::as_str),
                Some("first")
            );
            break;
        }
    }
    send_notification(
        &mut socket,
        json!({
            "channel": CHAT_URI,
            "clientSeq": 2,
            "action": { "type": "chat/turnCancelled", "turnId": "turn-cancel" },
        }),
    )
    .await?;

    loop {
        let action = next_action(&mut socket).await?;
        if action.pointer("/action/type").and_then(Value::as_str) == Some("chat/turnCancelled") {
            break;
        }
    }
    match timeout(Duration::from_millis(400), next_action(&mut socket)).await {
        Err(_) => {}
        Ok(Ok(action)) => {
            return Err(
                io::Error::other(format!("action arrived after cancellation: {action}")).into(),
            );
        }
        Ok(Err(error)) => return Err(error),
    }

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

#[tokio::test]
async fn invalid_chat_actions_are_rejected_without_wedging_the_client() -> TestResult {
    let fixture = start_fixture().await?;
    let host = start_host(&fixture).await?;
    let mut socket = connect_and_initialize(&host).await?;

    let malformed = json!({ "type": "chat/turnStarted", "turnId": "missing-fields" });
    send_action(&mut socket, 10, malformed.clone()).await?;
    expect_rejection(&mut socket, malformed, 10).await?;

    let accepted = json!({
        "type": "chat/turnStarted",
        "turnId": "active-turn",
        "startedAt": "2026-08-31T00:00:00.000Z",
        "message": { "text": "cancel", "origin": { "kind": "user" } },
    });
    send_action(&mut socket, 11, accepted).await?;
    wait_for_action(&mut socket, "chat/turnStarted", 0).await?;

    let unknown = json!({ "type": "chat/notSupported", "turnId": "active-turn" });
    send_action(&mut socket, 12, unknown.clone()).await?;
    expect_rejection(&mut socket, unknown, 12).await?;

    let second_start = json!({
        "type": "chat/turnStarted",
        "turnId": "second-turn",
        "startedAt": "2026-08-31T00:00:00.000Z",
        "message": { "text": "ignored", "origin": { "kind": "user" } },
    });
    send_action(&mut socket, 13, second_start.clone()).await?;
    expect_rejection(&mut socket, second_start, 13).await?;

    let mismatch = json!({ "type": "chat/turnCancelled", "turnId": "other-turn" });
    send_action(&mut socket, 14, mismatch.clone()).await?;
    expect_rejection(&mut socket, mismatch, 14).await?;

    send_action(
        &mut socket,
        15,
        json!({ "type": "chat/turnCancelled", "turnId": "active-turn" }),
    )
    .await?;
    wait_for_action(&mut socket, "chat/turnCancelled", 0).await?;

    socket.close(None).await?;
    host.shutdown().await;
    fixture.shutdown().await;
    Ok(())
}

async fn start_host(fixture: &Fixture) -> TestResult<RunningHost> {
    start_host_config(fixture, false).await
}

async fn start_host_with_chaining(fixture: &Fixture) -> TestResult<RunningHost> {
    start_host_config(fixture, true).await
}

async fn start_host_config(fixture: &Fixture, response_chaining: bool) -> TestResult<RunningHost> {
    Ok(start(HostConfig {
        bind: "127.0.0.1:0".parse::<SocketAddr>()?,
        openai_base_url: fixture.base_url.clone(),
        openai_model: "fixture-model".to_owned(),
        openai_api_key: None,
        response_chaining,
    })
    .await?)
}

async fn wait_for_turn_complete(socket: &mut Socket, turn_id: &str) -> TestResult {
    loop {
        let action = next_action(socket).await?;
        if action.pointer("/action/type").and_then(Value::as_str) == Some("chat/error") {
            return Err(io::Error::other(format!("turn {turn_id} failed: {action}")).into());
        }
        if action.pointer("/action/type").and_then(Value::as_str) == Some("chat/turnComplete")
            && action.pointer("/action/turnId").and_then(Value::as_str) == Some(turn_id)
        {
            return Ok(());
        }
    }
}

async fn connect_and_initialize(host: &RunningHost) -> TestResult<Socket> {
    let (mut socket, _) = connect_async(host.websocket_url()).await?;
    assert_eq!(
        request(&mut socket, 1, "ping", json!({ "channel": "ahp-root://" }))
            .await?
            .pointer("/result"),
        Some(&Value::Null)
    );
    let response = initialize(&mut socket, "test-client", json!(["ahp-root://"])).await?;
    let root: ahp_types::state::Snapshot = serde_json::from_value(
        response
            .pointer("/result/snapshots/0")
            .cloned()
            .ok_or_else(|| io::Error::other("initialize did not return its root snapshot"))?,
    )?;
    assert_eq!(root.resource, "ahp-root://");
    for (id, channel) in [(3, SESSION_URI), (4, CHAT_URI)] {
        let response = request(&mut socket, id, "subscribe", json!({ "channel": channel })).await?;
        let _: ahp_types::state::Snapshot = serde_json::from_value(
            response
                .pointer("/result/snapshot")
                .cloned()
                .ok_or_else(|| io::Error::other("subscribe did not return a snapshot"))?,
        )?;
    }
    let sessions = request(
        &mut socket,
        5,
        "listSessions",
        json!({ "channel": "ahp-root://" }),
    )
    .await?;
    let _: ahp_types::state::SessionSummary = serde_json::from_value(
        sessions
            .pointer("/result/items/0")
            .cloned()
            .ok_or_else(|| io::Error::other("listSessions did not return the fixed session"))?,
    )?;
    assert_eq!(
        request(&mut socket, 6, "ping", json!({ "channel": "ahp-root://" }))
            .await?
            .pointer("/result"),
        Some(&Value::Null)
    );
    Ok(socket)
}

async fn start_turn(socket: &mut Socket, prompt: &str, turn_id: &str) -> TestResult {
    send_action(
        socket,
        1,
        json!({
            "type": "chat/turnStarted",
            "turnId": turn_id,
            "startedAt": "2026-08-31T00:00:00.000Z",
            "message": { "text": prompt, "origin": { "kind": "user" } },
        }),
    )
    .await
}

async fn initialize(
    socket: &mut Socket,
    client_id: &str,
    subscriptions: Value,
) -> TestResult<Value> {
    request(
        socket,
        2,
        "initialize",
        json!({
            "channel": "ahp-root://",
            "clientId": client_id,
            "protocolVersions": ["0.9.0"],
            "initialSubscriptions": subscriptions,
        }),
    )
    .await
}

async fn send_action(socket: &mut Socket, client_seq: i64, action: Value) -> TestResult {
    send_notification(
        socket,
        json!({ "channel": CHAT_URI, "clientSeq": client_seq, "action": action }),
    )
    .await
}

async fn send_notification(socket: &mut Socket, params: Value) -> TestResult {
    socket
        .send(Message::Text(
            json!({ "jsonrpc": "2.0", "method": "dispatchAction", "params": params })
                .to_string()
                .into(),
        ))
        .await?;
    Ok(())
}

async fn request(socket: &mut Socket, id: u64, method: &str, params: Value) -> TestResult<Value> {
    socket
        .send(Message::Text(
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
                .to_string()
                .into(),
        ))
        .await?;
    loop {
        let frame = next_frame(socket).await?;
        let Message::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("id") == Some(&Value::from(id)) {
            return Ok(value);
        }
    }
}

async fn next_action(socket: &mut Socket) -> TestResult<Value> {
    loop {
        let frame = next_frame(socket).await?;
        let Message::Text(text) = frame else {
            continue;
        };
        let value: Value = serde_json::from_str(&text)?;
        if value.get("method").and_then(Value::as_str) == Some("action") {
            return value
                .get("params")
                .cloned()
                .ok_or_else(|| io::Error::other("action notification had no params").into());
        }
    }
}

async fn wait_for_action(socket: &mut Socket, action_type: &str, from_seq: i64) -> TestResult {
    loop {
        let action = next_action(socket).await?;
        let sequence = action
            .get("serverSeq")
            .and_then(Value::as_i64)
            .ok_or_else(|| io::Error::other("action had no serverSeq"))?;
        assert!(sequence > from_seq);
        if action.pointer("/action/type").and_then(Value::as_str) == Some(action_type) {
            return Ok(());
        }
    }
}

async fn expect_rejection(socket: &mut Socket, expected: Value, client_seq: i64) -> TestResult {
    loop {
        let action = next_action(socket).await?;
        if action
            .get("rejectionReason")
            .and_then(Value::as_str)
            .is_none()
        {
            continue;
        }
        assert_eq!(action.get("action"), Some(&expected));
        assert_eq!(
            action.pointer("/origin/clientId").and_then(Value::as_str),
            Some("test-client")
        );
        assert_eq!(
            action.pointer("/origin/clientSeq").and_then(Value::as_i64),
            Some(client_seq)
        );
        return Ok(());
    }
}

async fn next_frame(socket: &mut Socket) -> TestResult<Message> {
    timeout(Duration::from_secs(2), socket.next())
        .await
        .map_err(|_| io::Error::other("timed out waiting for WebSocket frame"))?
        .ok_or_else(|| io::Error::other("WebSocket closed"))?
        .map_err(Into::into)
}

async fn start_fixture() -> TestResult<Fixture> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let app = Router::new().route("/v1/responses", post(responses));
    let (shutdown, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = receiver.await;
            })
            .await;
    });
    Ok(Fixture {
        base_url: format!("http://{address}"),
        shutdown,
        task,
    })
}

async fn responses(Json(request): Json<Value>) -> Response {
    let input = request.get("input").and_then(Value::as_array);
    let prompt = input.and_then(|items| {
        items.iter().rev().find_map(|item| {
            (item.get("role").and_then(Value::as_str) == Some("user"))
                .then(|| item.get("content").and_then(Value::as_str))
                .flatten()
        })
    });
    if request.get("stream") != Some(&Value::Bool(true))
        || input.is_none_or(Vec::is_empty)
        || prompt.is_none()
    {
        return (StatusCode::BAD_REQUEST, "expected streamed user input").into_response();
    }
    let previous_response_id = request.get("previous_response_id").and_then(Value::as_str);
    let valid_chain = match prompt {
        Some("chain-second" | "cancel-chain") => previous_response_id == Some("resp-first"),
        Some("after-cancel") => {
            previous_response_id.is_none()
                && input.is_some_and(|items| {
                    items.as_slice()
                        == [
                            json!({ "role": "user", "content": "first" }),
                            json!({ "role": "assistant", "content": "firstsecond" }),
                            json!({ "role": "user", "content": "cancel-chain" }),
                            json!({ "role": "assistant", "content": "first" }),
                            json!({ "role": "user", "content": "after-cancel" }),
                        ]
                })
        }
        _ => previous_response_id.is_none(),
    };
    if !valid_chain {
        return (
            StatusCode::BAD_REQUEST,
            "unexpected previous_response_id for prompt",
        )
            .into_response();
    }
    let response_id = match prompt {
        Some("chain-second") => "resp-second",
        Some("cancel-chain") => "resp-cancelled",
        Some("after-cancel") => "resp-after-cancel",
        _ => {
            if previous_response_id.is_some() {
                return (
                    StatusCode::BAD_REQUEST,
                    "unexpected previous_response_id for initial request",
                )
                    .into_response();
            }
            "resp-first"
        }
    };
    if request.get("previous_response_id").is_some() && previous_response_id.is_none() {
        return (
            StatusCode::BAD_REQUEST,
            "previous_response_id must be a string",
        )
            .into_response();
    }
    let delay = if matches!(prompt, Some("cancel" | "cancel-chain")) {
        Duration::from_millis(250)
    } else {
        Duration::from_millis(10)
    };
    let frames = vec![
        (
            Duration::ZERO,
            format!(
                "event: response.created\ndata: {{\"type\":\"response.created\",\"response\":{{\"id\":\"{response_id}\"}}}}\n\n"
            ),
        ),
        (
            Duration::ZERO,
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"first\"}\n\n".to_owned(),
        ),
        (
            delay,
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"second\"}\n\n".to_owned(),
        ),
        (
            Duration::ZERO,
            "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n".to_owned(),
        ),
    ];
    let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
        let (delay, frame) = frames.next()?;
        sleep(delay).await;
        Some((Ok::<Bytes, Infallible>(Bytes::from(frame)), frames))
    });
    (
        [(CONTENT_TYPE, "text/event-stream")],
        Body::from_stream(stream),
    )
        .into_response()
}
