//! Session catalogue command implementations.

use ahp_types::commands::{
    CreateChatParams, CreateSessionParams, DisposeSessionParams, InitializeParams,
    ListSessionsParams, ReconnectParams,
};
use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::errors::ahp_error_codes;
use ahp_types::SUPPORTED_PROTOCOL_VERSIONS;
use serde_json::{json, Value};
use uuid::Uuid;

use super::snapshot;
use crate::host::CoreHost;
use crate::rpc::RpcFault;
use crate::store::{ActionReplay, CreateChat, CreateSession, MarkSessionReady};

mod config;

pub(super) use config::{
    create_session_config, resolve_session_config, session_config_completions,
};

pub(super) async fn initialize(
    host: &CoreHost,
    peer_id: u64,
    request: InitializeParams,
) -> Result<Value, RpcFault> {
    if host.peer_initialized(peer_id).await {
        return Err(RpcFault {
            code: ahp_types::errors::json_rpc_error_codes::INVALID_REQUEST,
            message: "initialize can only be sent once per connection".into(),
            data: None,
        });
    }
    if request.channel != ROOT_RESOURCE_URI || request.client_id.trim().is_empty() {
        return Err(RpcFault::invalid_params(
            "initialize requires ahp-root:// and a non-empty clientId",
        ));
    }
    let Some(version) = request
        .protocol_versions
        .iter()
        .find(|version| SUPPORTED_PROTOCOL_VERSIONS.contains(&version.as_str()))
        .cloned()
    else {
        return Err(RpcFault {
            code: ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION,
            message: "no mutually supported AHP protocol version".into(),
            data: Some(json!({ "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS })),
        });
    };
    let subscriptions = request.initial_subscriptions.unwrap_or_default();
    let snapshots = subscriptions
        .iter()
        .cloned()
        .map(|channel| snapshot(host, channel))
        .collect::<Result<Vec<_>, _>>()?;
    host.set_peer_initialized(peer_id, request.client_id, subscriptions)
        .await;
    Ok(json!({
        "protocolVersion": version,
        "serverSeq": host.inner.store.server_sequence().map_err(RpcFault::store)?,
        "serverInfo": { "name": host.inner.config.server_name, "version": env!("CARGO_PKG_VERSION") },
        "snapshots": snapshots,
    }))
}

pub(super) async fn reconnect(
    host: &CoreHost,
    peer_id: u64,
    request: ReconnectParams,
) -> Result<Value, RpcFault> {
    if host.peer_initialized(peer_id).await {
        return Err(RpcFault {
            code: ahp_types::errors::json_rpc_error_codes::INVALID_REQUEST,
            message: "reconnect must be the first request on a connection".into(),
            data: None,
        });
    }
    if request.channel != ROOT_RESOURCE_URI
        || request.client_id.trim().is_empty()
        || request.last_seen_server_seq < 0
    {
        return Err(RpcFault::invalid_params("reconnect requires ahp-root://, a non-empty clientId, and non-negative lastSeenServerSeq"));
    }
    let mut available = Vec::new();
    let mut missing = Vec::new();
    for channel in request.subscriptions {
        match snapshot(host, channel.clone()) {
            Ok(_) => available.push(channel),
            Err(fault)
                if matches!(
                    fault.code,
                    ahp_error_codes::SESSION_NOT_FOUND | ahp_error_codes::NOT_FOUND
                ) =>
            {
                missing.push(channel);
            }
            Err(fault) => return Err(fault),
        }
    }
    let replay = host
        .inner
        .store
        .replay_since(request.last_seen_server_seq)
        .map_err(RpcFault::store)?;
    host.set_peer_initialized(peer_id, request.client_id, available.clone())
        .await;
    match replay {
        ActionReplay::Replay(actions) => Ok(
            json!({ "type": "replay", "actions": actions.into_iter().filter(|item| available.contains(&item.channel)).collect::<Vec<_>>(), "missing": missing }),
        ),
        ActionReplay::SnapshotRequired => Ok(
            json!({ "type": "snapshot", "snapshots": available.into_iter().map(|channel| snapshot(host, channel)).collect::<Result<Vec<_>, _>>()? }),
        ),
    }
}

pub(super) fn list_sessions(
    host: &CoreHost,
    request: ListSessionsParams,
) -> Result<Value, RpcFault> {
    if request.channel != ROOT_RESOURCE_URI {
        return Err(RpcFault::invalid_params(
            "listSessions must target ahp-root://",
        ));
    }
    let items = host.inner.store.list_sessions().map_err(RpcFault::store)?;
    let offset = request.cursor.as_deref().map_or(Ok(0), |cursor| {
        cursor
            .strip_prefix("offset:")
            .and_then(|offset| offset.parse::<usize>().ok())
            .filter(|offset| *offset <= items.len())
            .ok_or_else(|| RpcFault::invalid_params("unrecognised session-list cursor"))
    })?;
    let limit = request.limit.map_or(Ok(100_usize), |limit| {
        usize::try_from(limit)
            .ok()
            .filter(|limit| *limit > 0)
            .map(|limit| limit.min(100))
            .ok_or_else(|| RpcFault::invalid_params("session-list limit must be positive"))
    })?;
    let end = offset.saturating_add(limit).min(items.len());
    Ok(
        json!({ "items": items[offset..end], "nextCursor": (end < items.len()).then(|| format!("offset:{end}")) }),
    )
}

pub(super) async fn create_session(
    host: &CoreHost,
    request: CreateSessionParams,
) -> Result<Value, RpcFault> {
    if !request.channel.starts_with("ahp-session:/") {
        return Err(RpcFault::invalid_params(
            "createSession channel must be an ahp-session URI",
        ));
    }
    if request.active_client.is_some() || request.fork.is_some() {
        return Err(RpcFault::invalid_params(
            "active clients and forks are unsupported",
        ));
    }
    let provider = request.provider.unwrap_or_else(|| "openai".into());
    if provider != "openai" {
        return Err(RpcFault {
            code: ahp_error_codes::PROVIDER_NOT_FOUND,
            message: format!("no agent registered for provider {provider:?}"),
            data: None,
        });
    }
    let workspace = host
        .inner
        .profile
        .select_workspace(request.working_directories)
        .map_err(RpcFault::internal)?;
    let model_selection = host
        .inner
        .model_catalog
        .resolve(create_session_config(request.config.as_ref())?)
        .map_err(|error| RpcFault::invalid_params(error.to_string()))?;
    let transition = host
        .inner
        .store
        .create_session(CreateSession {
            session: request.channel,
            default_chat: format!("ahp-chat:/{}", Uuid::new_v4()),
            provider,
            workspace: workspace.uri().clone(),
            model_selection,
        })
        .map_err(RpcFault::store)?;
    let session = transition.value.session.clone();
    host.fanout(&transition).await;
    let summary = host
        .inner
        .store
        .session_summary(&session)
        .map_err(RpcFault::store)?;
    host.broadcast_notification(
        ROOT_RESOURCE_URI,
        "root/sessionAdded",
        json!({ "channel": ROOT_RESOURCE_URI, "summary": summary }),
    )
    .await;
    let ready = host
        .inner
        .store
        .mark_session_ready(MarkSessionReady { session })
        .map_err(RpcFault::store)?;
    host.fanout(&ready).await;
    Ok(Value::Null)
}

pub(super) async fn create_chat(
    host: &CoreHost,
    request: CreateChatParams,
) -> Result<Value, RpcFault> {
    if !request.channel.starts_with("ahp-session:/") || !request.chat.starts_with("ahp-chat:/") {
        return Err(RpcFault::invalid_params(
            "createChat requires AHP session and chat URIs",
        ));
    }
    if request.source.is_some()
        || request.initial_message.is_some()
        || request.working_directories.is_some()
    {
        return Err(RpcFault::invalid_params(
            "chat sources, initial messages, and per-chat workspaces are unsupported",
        ));
    }
    let transition = host
        .inner
        .store
        .create_chat(CreateChat {
            session: request.channel,
            chat: request.chat,
        })
        .map_err(RpcFault::store)?;
    host.fanout(&transition).await;
    Ok(Value::Null)
}

pub(super) async fn dispose_session(
    host: &CoreHost,
    request: DisposeSessionParams,
) -> Result<Value, RpcFault> {
    let transition = host
        .inner
        .store
        .dispose_session(&request.channel)
        .map_err(RpcFault::store)?;
    let disposed = transition.value.clone();
    let mut cancellations = host.inner.cancellations.lock().await;
    cancellations.retain(|key, token| {
        let remove = disposed.chats.contains(&key.chat);
        if remove {
            token.cancel();
        }
        !remove
    });
    host.inner
        .approvals
        .lock()
        .await
        .retain(|key, _| !disposed.chats.contains(&key.chat));
    let mut peers = host.inner.peers.lock().await;
    for peer in peers.values_mut() {
        peer.subscriptions.remove(&disposed.session);
        for chat in &disposed.chats {
            peer.subscriptions.remove(chat);
        }
    }
    drop(peers);
    drop(cancellations);
    host.fanout(&transition).await;
    host.broadcast_notification(
        ROOT_RESOURCE_URI,
        "root/sessionRemoved",
        json!({ "channel": ROOT_RESOURCE_URI, "session": disposed.session }),
    )
    .await;
    Ok(Value::Null)
}
