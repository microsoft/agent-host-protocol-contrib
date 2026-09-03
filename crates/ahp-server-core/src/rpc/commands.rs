//! Request-method dispatch and common subscription helpers.

mod session;

use ahp_types::commands::{SubscribeParams, UnsubscribeParams};
use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::errors::json_rpc_error_codes;
use ahp_types::messages::{JsonRpcNotification, JsonRpcRequest};
use serde_json::{json, Value};

use super::{params, RpcFault};
use crate::host::CoreHost;
use crate::store::SnapshotRequest;

pub(super) async fn process_request(
    host: &CoreHost,
    peer_id: u64,
    request: JsonRpcRequest,
) -> Result<Value, RpcFault> {
    match request.method.as_str() {
        "initialize" => session::initialize(host, peer_id, params(&request)?).await,
        "reconnect" => session::reconnect(host, peer_id, params(&request)?).await,
        "ping" => {
            let value: Value = params(&request)?;
            if value.get("channel").and_then(Value::as_str) != Some(ROOT_RESOURCE_URI) {
                return Err(RpcFault::invalid_params("ping must target ahp-root://"));
            }
            Ok(Value::Null)
        }
        "subscribe" => {
            require_initialized(host, peer_id).await?;
            let request: SubscribeParams = params(&request)?;
            let snapshot = snapshot(host, request.channel.clone())?;
            add_subscriptions(host, peer_id, [request.channel]).await;
            Ok(json!({ "snapshot": snapshot }))
        }
        "listSessions" => {
            require_initialized(host, peer_id).await?;
            session::list_sessions(host, params(&request)?)
        }
        "resolveSessionConfig" => {
            require_initialized(host, peer_id).await?;
            session::resolve_session_config(host, params(&request)?)
        }
        "sessionConfigCompletions" => {
            require_initialized(host, peer_id).await?;
            session::session_config_completions(host, params(&request)?)
        }
        "createSession" => {
            require_initialized(host, peer_id).await?;
            session::create_session(host, params(&request)?).await
        }
        "createChat" => {
            require_initialized(host, peer_id).await?;
            session::create_chat(host, params(&request)?).await
        }
        "disposeSession" => {
            require_initialized(host, peer_id).await?;
            session::dispose_session(host, params(&request)?).await
        }
        "fetchTurns" => {
            require_initialized(host, peer_id).await?;
            session::fetch_turns(host, params(&request)?)
        }
        _ => Err(RpcFault {
            code: json_rpc_error_codes::METHOD_NOT_FOUND,
            message: format!(
                "method {:?} is not supported by this AHP host",
                request.method
            ),
            data: None,
        }),
    }
}

pub(super) async fn process_notification(
    host: &CoreHost,
    peer_id: u64,
    notification: JsonRpcNotification,
) -> Result<(), crate::CoreHostError> {
    match notification.method.as_str() {
        "unsubscribe" => {
            if let Some(params) = notification
                .params
                .and_then(|params| serde_json::from_value::<UnsubscribeParams>(params).ok())
            {
                host.unsubscribe(peer_id, &params.channel).await;
            }
        }
        "dispatchAction" if host.peer_initialized(peer_id).await => {
            if let Some(params) = notification.params {
                crate::turns::dispatch_action(host, peer_id, params).await;
            }
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn snapshot(
    host: &CoreHost,
    channel: String,
) -> Result<ahp_types::state::Snapshot, RpcFault> {
    host.inner
        .store
        .snapshot(SnapshotRequest {
            channel,
            model_catalog: host.inner.model_catalog.clone(),
            server_tools: host.inner.profile.server_tools(),
        })
        .map_err(RpcFault::store)
}

pub(super) async fn add_subscriptions(
    host: &CoreHost,
    peer_id: u64,
    channels: impl IntoIterator<Item = String>,
) {
    let channels = channels.into_iter().collect::<Vec<_>>();
    if let Some(peer) = host.inner.peers.lock().await.get_mut(&peer_id) {
        peer.subscriptions.extend(channels);
    }
}

async fn require_initialized(host: &CoreHost, peer_id: u64) -> Result<(), RpcFault> {
    if host.peer_initialized(peer_id).await {
        Ok(())
    } else {
        Err(RpcFault {
            code: json_rpc_error_codes::INVALID_REQUEST,
            message: "initialize or reconnect must be the first request".into(),
            data: None,
        })
    }
}
