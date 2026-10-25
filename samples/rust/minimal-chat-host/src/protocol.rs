// Flow: turn JSON-RPC into fixed snapshots and the two supported chat actions.

use ahp_types::{
    common::ROOT_RESOURCE_URI,
    errors::{ahp_error_codes, json_rpc_error_codes},
    negotiate_protocol_version, SUPPORTED_PROTOCOL_VERSIONS,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    host::{session_summary, snapshot, start_turn, AppState, Connection, Peer, CHAT_URI},
    responses,
};

#[rustfmt::skip] #[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Incoming { jsonrpc: String, id: Option<u64>, method: String, params: Option<Value> }
#[rustfmt::skip] struct RpcFault { code: i32, message: String, data: Option<Value> }

impl RpcFault {
    fn new(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

pub(crate) async fn receive_json_rpc(state: &AppState, connection: &mut Connection, text: &str) {
    let incoming = match serde_json::from_str::<Incoming>(text) {
        Ok(incoming) => incoming,
        Err(error) => {
            send(
                state,
                connection.peer_id,
                json!({ "jsonrpc": "2.0", "id": Value::Null, "error": {
                    "code": json_rpc_error_codes::PARSE_ERROR,
                    "message": format!("invalid JSON-RPC message: {error}"),
                }}),
            )
            .await;
            return;
        }
    };
    if incoming.jsonrpc != "2.0" {
        if let Some(id) = incoming.id {
            send(
                state,
                connection.peer_id,
                json!({ "jsonrpc": "2.0", "id": id, "error": {
                    "code": json_rpc_error_codes::INVALID_REQUEST,
                    "message": "jsonrpc must be 2.0",
                }}),
            )
            .await;
        }
        return;
    }
    if let Some(id) = incoming.id {
        // Snapshot, subscription, and this queued reply share one publication boundary.
        let _publication = state.publication.lock().await;
        let reply = match request(state, connection, incoming).await {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(fault) => error_response(Value::from(id), fault),
        };
        send(state, connection.peer_id, reply).await;
    } else if incoming.method == "dispatchAction" && connection.initialized {
        if let Some(params) = incoming.params {
            dispatch_action(state, connection, &params).await;
        }
    }
}

async fn request(
    state: &AppState,
    connection: &mut Connection,
    incoming: Incoming,
) -> Result<Value, RpcFault> {
    if incoming.method == "reconnect" {
        return Err(method_not_found("reconnect is intentionally unsupported"));
    }
    if !matches!(
        incoming.method.as_str(),
        "ping" | "initialize" | "subscribe" | "listSessions"
    ) {
        return Err(method_not_found(&format!(
            "{} is outside this minimal sample",
            incoming.method
        )));
    }
    let params = incoming
        .params
        .ok_or_else(|| invalid_params("request requires params"))?;
    match incoming.method.as_str() {
        "ping" => {
            require_root(&params)?;
            Ok(Value::Null)
        }
        "initialize" => initialize(state, connection, &params).await,
        "subscribe" => {
            require_initialized(connection)?;
            let channel = channel(&params)?;
            let mut snapshots = subscribe(state, connection.peer_id, &[channel.to_owned()]).await?;
            snapshots
                .pop()
                .map(|snapshot| json!({ "snapshot": snapshot }))
                .ok_or_else(|| {
                    RpcFault::new(
                        json_rpc_error_codes::INTERNAL_ERROR,
                        "subscription did not create a snapshot",
                    )
                })
        }
        "listSessions" => {
            require_initialized(connection)?;
            require_root(&params)?;
            let shared = state.shared.lock().await;
            Ok(json!({ "items": [session_summary(&shared)] }))
        }
        _ => Err(method_not_found("method is outside this minimal sample")),
    }
}

async fn initialize(
    state: &AppState,
    connection: &mut Connection,
    params: &Value,
) -> Result<Value, RpcFault> {
    if connection.initialized {
        return Err(RpcFault::new(
            json_rpc_error_codes::INVALID_REQUEST,
            "initialize may be sent only once per connection",
        ));
    }
    require_root(params)?;
    let client_id = params
        .get("clientId")
        .and_then(Value::as_str)
        .filter(|client_id| !client_id.trim().is_empty())
        .ok_or_else(|| invalid_params("initialize requires a non-empty clientId"))?;
    let versions: Vec<String> = serde_json::from_value(
        params
            .get("protocolVersions")
            .cloned()
            .ok_or_else(|| invalid_params("initialize requires protocolVersions"))?,
    )
    .map_err(|_| invalid_params("protocolVersions must be an array of strings"))?;
    let version = negotiate_protocol_version(&versions)
        .map_err(|message| invalid_params(&message))?
        .ok_or_else(|| RpcFault {
            code: ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION,
            message: "no mutually supported AHP protocol version".to_owned(),
            data: Some(json!({ "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS })),
        })?;
    let subscriptions = params
        .get("initialSubscriptions")
        .map_or(Ok(Vec::new()), |value| {
            serde_json::from_value(value.clone())
                .map_err(|_| invalid_params("initialSubscriptions must be an array of strings"))
        })?;
    let snapshots = subscribe(state, connection.peer_id, &subscriptions).await?;
    connection.initialized = true;
    connection.client_id = Some(client_id.to_owned());
    let server_seq = state.shared.lock().await.server_seq;
    Ok(json!({
        "protocolVersion": version,
        "serverSeq": server_seq,
        "serverInfo": { "name": "minimal-chat-host", "version": env!("CARGO_PKG_VERSION") },
        "snapshots": snapshots,
    }))
}

async fn subscribe(
    state: &AppState,
    peer_id: u64,
    channels: &[String],
) -> Result<Vec<Value>, RpcFault> {
    let snapshots = {
        let shared = state.shared.lock().await;
        channels
            .iter()
            .map(|channel| {
                snapshot(&shared, &state.config.model, channel).ok_or_else(|| {
                    RpcFault::new(
                        ahp_error_codes::NOT_FOUND,
                        format!("this sample has no channel {channel:?}"),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut peers = state.peers.lock().await;
    let peer = peers.get_mut(&peer_id).ok_or_else(|| {
        RpcFault::new(
            json_rpc_error_codes::INTERNAL_ERROR,
            "connection disappeared while subscribing",
        )
    })?;
    peer.subscriptions.extend(channels.iter().cloned());
    Ok(snapshots)
}

async fn dispatch_action(state: &AppState, connection: &Connection, params: &Value) {
    let Some(channel) = params.get("channel").and_then(Value::as_str) else {
        return;
    };
    let Some(client_seq) = params.get("clientSeq").and_then(Value::as_i64) else {
        return;
    };
    let Some(action) = params.get("action") else {
        return;
    };
    if channel != CHAT_URI {
        return;
    }
    let origin = json!({
        "clientId": connection.client_id.as_deref().unwrap_or_default(),
        "clientSeq": client_seq,
    });
    match action.get("type").and_then(Value::as_str) {
        Some("chat/turnStarted") => start_turn(state, action, origin).await,
        Some("chat/turnCancelled") => cancel_turn(state, action, origin).await,
        _ => {
            state
                .reject(
                    action.clone(),
                    origin,
                    "action is outside this sample's supported lifecycle",
                )
                .await
        }
    }
}

async fn cancel_turn(state: &AppState, action: &Value, origin: Value) {
    let Some(turn_id) = action.get("turnId").and_then(Value::as_str) else {
        state
            .reject(
                action.clone(),
                origin,
                "chat/turnCancelled requires a matching active turnId",
            )
            .await;
        return;
    };
    if let Some(abort) = responses::finish_turn(
        state,
        turn_id,
        "cancelled",
        None,
        "chat/turnCancelled",
        Some(origin.clone()),
        None,
    )
    .await
    {
        abort.abort();
    } else {
        state
            .reject(
                action.clone(),
                origin,
                "chat/turnCancelled did not match the active turn",
            )
            .await;
    }
}

#[rustfmt::skip]
fn channel(params: &Value) -> Result<&str, RpcFault> { params.get("channel").and_then(Value::as_str).ok_or_else(|| invalid_params("command requires a string channel")) }
#[rustfmt::skip]
fn require_root(params: &Value) -> Result<(), RpcFault> { if channel(params)? == ROOT_RESOURCE_URI { Ok(()) } else { Err(invalid_params("command must target ahp-root://")) } }
#[rustfmt::skip]
fn require_initialized(connection: &Connection) -> Result<(), RpcFault> { if connection.initialized { Ok(()) } else { Err(RpcFault::new(json_rpc_error_codes::INVALID_REQUEST, "initialize must precede this request")) } }
#[rustfmt::skip]
fn invalid_params(message: impl Into<String>) -> RpcFault { RpcFault::new(json_rpc_error_codes::INVALID_PARAMS, message) }
#[rustfmt::skip]
fn method_not_found(message: &str) -> RpcFault { RpcFault::new(json_rpc_error_codes::METHOD_NOT_FOUND, message) }

fn error_response(id: Value, fault: RpcFault) -> Value {
    let mut error = json!({ "code": fault.code, "message": fault.message });
    if let Some(data) = fault.data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

async fn send(state: &AppState, peer_id: u64, message: Value) {
    if let Some(Peer { sender, .. }) = state.peers.lock().await.get(&peer_id) {
        let _ = sender.send(message.to_string());
    }
}
