//! Axum WebSocket transport and browser-Origin checks for `ahp-server-core`.

#![forbid(unsafe_code)]

mod origin;

use std::sync::Arc;

use ahp_server_core::CoreHost;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use thiserror::Error;

use origin::OriginPolicy;

/// Options for the WebSocket browser-origin policy.
#[derive(Clone, Debug, Default)]
pub struct WsOptions {
    /// Additional trusted `http://` or `https://` browser origins.
    ///
    /// A matching request `Host` is always trusted, and no-Origin native
    /// WebSocket clients are accepted.
    pub allowed_origins: Vec<String>,
}

/// Configuration failures while creating a WebSocket router.
#[derive(Debug, Error)]
pub enum WsError {
    /// An allowed Origin is not a bare HTTP(S) origin.
    #[error("invalid allowed origin {origin:?}: {reason}")]
    InvalidOrigin {
        /// Supplied configuration value.
        origin: String,
        /// Validation failure.
        reason: String,
    },
}

#[derive(Clone)]
struct WsState {
    host: CoreHost,
    origins: Arc<OriginPolicy>,
}

/// Builds the `/ahp` WebSocket transport router.
pub fn router(host: CoreHost, options: WsOptions) -> Result<Router, WsError> {
    let origins = OriginPolicy::new(options.allowed_origins)?;
    Ok(Router::new()
        .route("/ahp", get(upgrade))
        .with_state(WsState {
            host,
            origins: Arc::new(origins),
        }))
}

async fn upgrade(
    State(state): State<WsState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Response {
    if let Err(rejection) = state.origins.validate(&headers) {
        return (rejection.status, rejection.message).into_response();
    }
    websocket
        .on_upgrade(move |socket| connection_task(socket, state.host))
        .into_response()
}

async fn connection_task(socket: WebSocket, host: CoreHost) {
    let mut peer = host.open_peer().await;
    let peer_id = peer.id();
    let shutdown = host.shutdown();
    let (mut sender, mut receiver) = socket.split();
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            outbound = peer.recv() => {
                let Some(outbound) = outbound else { break };
                if sender.send(Message::Text(outbound.into())).await.is_err() { break; }
            }
            inbound = receiver.next() => {
                let Some(inbound) = inbound else { break };
                match inbound {
                    Ok(Message::Text(text)) => {
                        if host.handle_text(peer_id, text.as_str()).await.is_err() { break; }
                    }
                    Ok(Message::Close(_)) | Err(_) => break,
                    Ok(Message::Binary(_)) | Ok(Message::Ping(_)) | Ok(Message::Pong(_)) => {}
                }
            }
        }
    }
    host.close_peer(peer_id).await;
}
