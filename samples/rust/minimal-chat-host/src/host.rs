// Flow: compose the fixed in-memory host, WebSocket runtime, and shared state.

use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt::{self, Display, Formatter},
    net::SocketAddr,
    sync::Arc,
    time::Instant,
};

use ahp_types::common::ROOT_RESOURCE_URI;
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Response,
    routing::get,
    Router,
};
use chrono::{SecondsFormat, Utc};
use futures_util::{
    future::{AbortHandle, Abortable},
    SinkExt, StreamExt,
};
use reqwest::{Client, Url};
use serde_json::{json, Value};
use tokio::{
    net::TcpListener,
    sync::{mpsc, oneshot, Mutex},
    task::JoinHandle,
};

use crate::{protocol, responses};

pub const SESSION_URI: &str = "ahp-session:/minimal-chat-host";
pub const CHAT_URI: &str = "ahp-chat:/minimal-chat-host";

#[rustfmt::skip]
#[derive(Clone, Debug)]
pub struct HostConfig { pub bind: SocketAddr, pub openai_base_url: String, pub openai_model: String, pub openai_api_key: Option<String>, pub response_chaining: bool }

#[derive(Debug)]
pub struct HostError(pub(crate) String);

#[rustfmt::skip]
impl Display for HostError { fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result { formatter.write_str(&self.0) } }

impl Error for HostError {}

pub(crate) type HostResult<T> = Result<T, HostError>;

#[rustfmt::skip]
pub struct RunningHost { websocket_url: String, shutdown: oneshot::Sender<()>, task: JoinHandle<()> }
#[rustfmt::skip]
impl RunningHost {
    pub fn websocket_url(&self) -> &str { &self.websocket_url }
    pub async fn shutdown(self) { let _ = self.shutdown.send(()); let _ = self.task.await; }
}

#[rustfmt::skip]
#[derive(Clone)]
pub(crate) struct AppState { pub(crate) config: Arc<RuntimeConfig>, pub(crate) http: Client, pub(crate) shared: Arc<Mutex<Shared>>, pub(crate) peers: Arc<Mutex<HashMap<u64, Peer>>>, pub(crate) publication: Arc<Mutex<()>> }
#[rustfmt::skip]
pub(crate) struct RuntimeConfig { pub(crate) responses_url: Url, pub(crate) model: String, pub(crate) api_key: Option<String>, pub(crate) response_chaining: bool }
#[rustfmt::skip]
pub(crate) struct Shared { pub(crate) next_peer: u64, pub(crate) server_seq: i64, pub(crate) created_at: String, pub(crate) modified_at: String, pub(crate) turns: Vec<FinishedTurn>, pub(crate) active: Option<ActiveTurn>, pub(crate) response_id: Option<String> }
#[rustfmt::skip]
pub(crate) struct ActiveTurn { pub(crate) id: String, pub(crate) started_at: String, pub(crate) started: Instant, pub(crate) message: Value, pub(crate) response: String, pub(crate) abort: AbortHandle }
#[rustfmt::skip]
#[derive(Clone)]
pub(crate) struct FinishedTurn { pub(crate) id: String, pub(crate) started_at: String, pub(crate) duration: i64, pub(crate) message: Value, pub(crate) response: String, pub(crate) state: &'static str, pub(crate) error: Option<String> }
#[rustfmt::skip]
pub(crate) struct Peer { pub(crate) subscriptions: HashSet<String>, pub(crate) sender: mpsc::UnboundedSender<String> }
#[rustfmt::skip]
pub(crate) struct Connection { pub(crate) peer_id: u64, pub(crate) initialized: bool, pub(crate) client_id: Option<String> }

pub async fn start(config: HostConfig) -> HostResult<RunningHost> {
    let responses_url = responses::endpoint(&config.openai_base_url)?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|error| HostError(format!("could not bind AHP listener: {error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| HostError(format!("could not read AHP listener address: {error}")))?;
    let now = timestamp();
    let state = AppState {
        config: Arc::new(RuntimeConfig {
            responses_url,
            model: config.openai_model,
            api_key: config.openai_api_key,
            response_chaining: config.response_chaining,
        }),
        http: Client::new(),
        shared: Arc::new(Mutex::new(Shared {
            next_peer: 0,
            server_seq: 0,
            created_at: now.clone(),
            modified_at: now,
            turns: Vec::new(),
            active: None,
            response_id: None,
        })),
        peers: Arc::new(Mutex::new(HashMap::new())),
        publication: Arc::new(Mutex::new(())),
    };
    let app = Router::new()
        .route("/ahp", get(websocket))
        .with_state(state);
    let (shutdown, receiver) = oneshot::channel();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = receiver.await;
            })
            .await;
    });
    Ok(RunningHost {
        websocket_url: format!("ws://{address}/ahp"),
        shutdown,
        task,
    })
}

async fn websocket(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    upgrade.on_upgrade(move |socket| connection_task(state, socket))
}

async fn connection_task(state: AppState, socket: WebSocket) {
    let peer_id = {
        let mut shared = state.shared.lock().await;
        shared.next_peer += 1;
        shared.next_peer
    };
    let (mut sender, mut receiver) = socket.split();
    let (outgoing_sender, mut outgoing_receiver) = mpsc::unbounded_channel();
    state.peers.lock().await.insert(
        peer_id,
        Peer {
            subscriptions: HashSet::new(),
            sender: outgoing_sender,
        },
    );
    let writer = tokio::spawn(async move {
        while let Some(text) = outgoing_receiver.recv().await {
            if sender.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });
    let mut connection = Connection {
        peer_id,
        initialized: false,
        client_id: None,
    };
    while let Some(message) = receiver.next().await {
        match message {
            Ok(Message::Text(text)) => {
                protocol::receive_json_rpc(&state, &mut connection, &text).await
            }
            Ok(Message::Close(_)) | Err(_) => break,
            Ok(_) => {}
        }
    }
    state.peers.lock().await.remove(&peer_id);
    writer.abort();
}

#[rustfmt::skip]
pub(crate) async fn start_turn(state: &AppState, action: &Value, origin: Value) {
    let Some((turn_id, started_at, message, prompt)) = incoming_turn(action) else {
        state.reject(action.clone(), origin, "chat/turnStarted requires turnId, startedAt, and a user message").await;
        return;
    };
    let (abort, registration) = AbortHandle::new_pair();
    let _publication = state.publication.lock().await;
    let context = {
        let mut shared = state.shared.lock().await;
        if shared.active.is_some() {
            None
        } else {
            let history = shared.turns.clone();
            let response_id = shared.response_id.clone();
            shared.modified_at = timestamp();
            shared.active = Some(ActiveTurn { id: turn_id.clone(), started_at: started_at.clone(), started: Instant::now(), message: message.clone(), response: String::new(), abort });
            Some((history, response_id))
        }
    };
    let Some((history, response_id)) = context else {
        state.publish_locked(CHAT_URI, action.clone(), Some(origin), Some("a text turn is already active")).await;
        return;
    };
    state.publish_locked(CHAT_URI, json!({ "type": "chat/turnStarted", "turnId": turn_id, "startedAt": started_at, "message": message }), Some(origin), None).await;
    state.publish_locked(CHAT_URI, json!({ "type": "chat/responsePart", "turnId": turn_id, "part": markdown_part(&turn_id, "") }), None, None).await;
    drop(_publication);
    let state = state.clone();
    tokio::spawn(async move { let _ = Abortable::new(responses::run_turn(state, turn_id, prompt, history, response_id), registration).await; });
}

fn incoming_turn(action: &Value) -> Option<(String, String, Value, String)> {
    let turn_id = action.get("turnId")?.as_str()?.trim();
    let started_at = action.get("startedAt")?.as_str()?.trim();
    let message = action.get("message")?.clone();
    let prompt = message.get("text")?.as_str()?.to_owned();
    let is_user = message.pointer("/origin/kind").and_then(Value::as_str) == Some("user");
    (!turn_id.is_empty() && !started_at.is_empty() && is_user)
        .then(|| (turn_id.to_owned(), started_at.to_owned(), message, prompt))
}

impl AppState {
    // Callers hold `publication`, making state mutation and its action atomic.
    pub(crate) async fn publish_locked(
        &self,
        channel: &str,
        action: Value,
        origin: Option<Value>,
        rejection_reason: Option<&str>,
    ) {
        let server_seq = {
            let mut shared = self.shared.lock().await;
            shared.server_seq += 1;
            shared.server_seq
        };
        let mut params = json!({ "channel": channel, "action": action, "serverSeq": server_seq });
        if let Some(origin) = origin {
            params["origin"] = origin;
        }
        if let Some(rejection_reason) = rejection_reason {
            params["rejectionReason"] = Value::String(rejection_reason.to_owned());
        }
        let frame = json!({ "jsonrpc": "2.0", "method": "action", "params": params }).to_string();
        for peer in self.peers.lock().await.values() {
            if peer.subscriptions.contains(channel) {
                let _ = peer.sender.send(frame.clone());
            }
        }
    }

    pub(crate) async fn reject(&self, action: Value, origin: Value, reason: &str) {
        let _publication = self.publication.lock().await;
        self.publish_locked(CHAT_URI, action, Some(origin), Some(reason))
            .await;
    }
}

pub(crate) fn timestamp() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

#[rustfmt::skip]
pub(crate) fn snapshot(shared: &Shared, model: &str, resource: &str) -> Option<Value> {
    let state = match resource {
        ROOT_RESOURCE_URI => json!({"agents": [{"provider": "openai-responses", "displayName": "OpenAI Responses", "description": "Streams one user prompt through the Responses API.", "models": [{"id": model, "provider": "openai", "name": model}]}], "activeSessions": 1}),
        SESSION_URI => json!({"provider": "openai-responses", "title": "Minimal Responses chat", "status": status(shared), "activity": activity(shared), "lifecycle": "ready", "activeClients": [], "chats": [chat_summary(shared)], "defaultChat": CHAT_URI}),
        CHAT_URI => json!({"resource": CHAT_URI, "title": "Minimal Responses chat", "status": status(shared), "activity": activity(shared), "modifiedAt": shared.modified_at, "turns": shared.turns.iter().map(finished_turn).collect::<Vec<_>>(), "activeTurn": shared.active.as_ref().map(active_turn)}),
        _ => return None,
    };
    Some(json!({"resource": resource, "state": state, "fromSeq": shared.server_seq}))
}

#[rustfmt::skip]
pub(crate) fn session_summary(shared: &Shared) -> Value {
    json!({"provider": "openai-responses", "title": "Minimal Responses chat", "status": status(shared), "activity": activity(shared), "resource": SESSION_URI, "createdAt": shared.created_at, "modifiedAt": shared.modified_at})
}

#[rustfmt::skip]
fn chat_summary(shared: &Shared) -> Value {
    json!({"resource": CHAT_URI, "title": "Minimal Responses chat", "status": status(shared), "activity": activity(shared), "modifiedAt": shared.modified_at})
}

#[rustfmt::skip]
fn active_turn(turn: &ActiveTurn) -> Value {
    json!({"id": turn.id, "startedAt": turn.started_at, "message": turn.message, "responseParts": [markdown_part(&turn.id, &turn.response)]})
}

#[rustfmt::skip]
fn finished_turn(turn: &FinishedTurn) -> Value {
    let mut response_parts = vec![markdown_part(&turn.id, &turn.response)];
    if let Some(error) = &turn.error {
        response_parts.push(json!({
            "kind": "error",
            "error": { "errorType": "upstream", "message": error },
        }));
    }
    json!({"id": turn.id, "startedAt": turn.started_at, "duration": turn.duration, "message": turn.message, "responseParts": response_parts, "state": turn.state})
}

#[rustfmt::skip]
pub(crate) fn markdown_part(turn_id: &str, content: &str) -> Value { json!({ "kind": "markdown", "id": format!("{turn_id}/text"), "content": content }) }
#[rustfmt::skip]
fn status(shared: &Shared) -> u32 { if shared.active.is_some() { 8 } else { 1 } }
#[rustfmt::skip]
fn activity(shared: &Shared) -> Option<&'static str> { shared.active.as_ref().map(|_| "Waiting for OpenAI Responses") }
