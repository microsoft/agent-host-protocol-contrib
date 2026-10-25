//! JSON-RPC frame validation and response envelopes.

mod commands;

use ahp_types::errors::{ahp_error_codes, json_rpc_error_codes};
use ahp_types::messages::{JsonRpcMessage, JsonRpcRequest, JsonRpcSuccessResponse};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::host::CoreHost;
use crate::store::StoreConflict;

pub(super) struct RpcFault {
    pub(super) code: i32,
    pub(super) message: String,
    pub(super) data: Option<Value>,
}

impl RpcFault {
    pub(super) fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: json_rpc_error_codes::INVALID_PARAMS,
            message: message.into(),
            data: None,
        }
    }

    pub(super) fn internal(error: impl ToString) -> Self {
        Self {
            code: json_rpc_error_codes::INTERNAL_ERROR,
            message: "AHP host internal error".into(),
            data: Some(json!({ "detail": error.to_string() })),
        }
    }

    pub(super) fn not_found(channel: impl Into<String>) -> Self {
        let channel = channel.into();
        let code = if channel.starts_with("ahp-session:/") {
            ahp_error_codes::SESSION_NOT_FOUND
        } else {
            ahp_error_codes::NOT_FOUND
        };
        Self {
            code,
            message: format!("unknown channel {channel:?}"),
            data: None,
        }
    }

    pub(super) fn store(error: crate::store::StoreError) -> Self {
        match error {
            crate::store::StoreError::Conflict(StoreConflict::UnknownChannel(channel)) => {
                Self::not_found(channel)
            }
            crate::store::StoreError::Conflict(StoreConflict::SessionExists(_)) => Self {
                code: ahp_error_codes::SESSION_ALREADY_EXISTS,
                message: error.to_string(),
                data: None,
            },
            crate::store::StoreError::Conflict(StoreConflict::ChatExists(_)) => Self {
                code: ahp_error_codes::ALREADY_EXISTS,
                message: error.to_string(),
                data: None,
            },
            crate::store::StoreError::Conflict(StoreConflict::ActiveTurn(_)) => Self {
                code: ahp_error_codes::TURN_IN_PROGRESS,
                message: error.to_string(),
                data: None,
            },
            _ => Self::internal(error),
        }
    }
}

pub(crate) async fn handle_text(
    host: &CoreHost,
    peer_id: u64,
    text: &str,
) -> Result<(), crate::CoreHostError> {
    let raw = match serde_json::from_str::<Value>(text) {
        Ok(raw) => raw,
        Err(_) => {
            return send_error(
                host,
                peer_id,
                Value::Null,
                json_rpc_error_codes::PARSE_ERROR,
                "invalid JSON-RPC message",
                None,
            )
            .await
        }
    };
    let response_id = raw.get("id").cloned().unwrap_or(Value::Null);
    let message = match serde_json::from_value::<JsonRpcMessage>(raw) {
        Ok(message) => message,
        Err(_) => {
            return send_error(
                host,
                peer_id,
                response_id,
                json_rpc_error_codes::INVALID_REQUEST,
                "invalid JSON-RPC request",
                None,
            )
            .await
        }
    };
    match message {
        JsonRpcMessage::Request(request) => {
            // Resource I/O does not publish actions; watch events take this gate
            // separately when committing and broadcasting their ordered envelopes.
            let _guard = if crate::resources::is_request(&request.method) {
                None
            } else {
                Some(host.inner.publish_lock.lock().await)
            };
            match commands::process_request(host, peer_id, request.clone()).await {
                Ok(result) => {
                    host.send_message(
                        peer_id,
                        JsonRpcMessage::SuccessResponse(JsonRpcSuccessResponse {
                            jsonrpc: Default::default(),
                            id: request.id,
                            result,
                        }),
                    )
                    .await
                }
                Err(error) => {
                    send_error(
                        host,
                        peer_id,
                        Value::from(request.id),
                        error.code,
                        &error.message,
                        error.data,
                    )
                    .await?;
                    if error.code == ahp_error_codes::UNSUPPORTED_PROTOCOL_VERSION {
                        drop(_guard);
                        host.close_peer(peer_id).await;
                    }
                    Ok(())
                }
            }
        }
        JsonRpcMessage::Notification(notification) => {
            commands::process_notification(host, peer_id, notification).await
        }
        JsonRpcMessage::SuccessResponse(_) | JsonRpcMessage::ErrorResponse(_) => {
            send_error(
                host,
                peer_id,
                Value::Null,
                json_rpc_error_codes::INVALID_REQUEST,
                "clients must send requests or notifications",
                None,
            )
            .await
        }
    }
}

pub(super) fn params<T: DeserializeOwned>(request: &JsonRpcRequest) -> Result<T, RpcFault> {
    request
        .params
        .clone()
        .ok_or_else(|| RpcFault::invalid_params("request requires params"))
        .and_then(|params| {
            serde_json::from_value(params)
                .map_err(|error| RpcFault::invalid_params(error.to_string()))
        })
}

async fn send_error(
    host: &CoreHost,
    peer_id: u64,
    id: Value,
    code: i32,
    message: &str,
    data: Option<Value>,
) -> Result<(), crate::CoreHostError> {
    let id = matches!(id, Value::Number(_) | Value::String(_))
        .then_some(id)
        .unwrap_or(Value::Null);
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    host.send_value(
        peer_id,
        json!({ "jsonrpc": "2.0", "id": id, "error": error }),
    )
    .await
}
