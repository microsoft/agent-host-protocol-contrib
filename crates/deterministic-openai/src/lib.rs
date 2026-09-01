//! Deterministic local OpenAI-compatible HTTP fixture for integration tests.
//!
//! This crate implements a deliberately small, stable subset of the OpenAI
//! Chat Completions and Responses endpoints. It is a test fixture, not an
//! OpenAI implementation.

mod chat_stream;
mod contract;
mod handlers;
mod nonstream;
mod request;
mod request_chat;
mod request_responses;
mod response;
mod responses_stream;
mod v1;
mod v2;
mod v2_json;
mod v2_state;

use std::net::SocketAddr;

use axum::routing::post;
use axum::Router;
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

/// Errors starting or stopping the deterministic fixture.
#[derive(Debug, Error)]
pub enum FixtureError {
    /// The HTTP listener could not be bound.
    #[error("failed to bind deterministic fixture: {0}")]
    Bind(#[from] std::io::Error),
    /// Axum stopped with an error.
    #[error("deterministic fixture failed: {0}")]
    Serve(String),
    /// The server task did not finish cleanly.
    #[error("deterministic fixture task failed: {0}")]
    Join(#[from] tokio::task::JoinError),
}

/// A running deterministic local OpenAI-compatible endpoint.
pub struct RunningFixture {
    address: SocketAddr,
    shutdown: CancellationToken,
    task: JoinHandle<Result<(), FixtureError>>,
}

impl RunningFixture {
    /// Starts a fixture supporting both Chat Completions and Responses APIs.
    pub async fn start(bind: SocketAddr) -> Result<Self, FixtureError> {
        Self::start_with_responses(bind, true).await
    }

    /// Starts a Chat Completions-only fixture for API fallback tests.
    pub async fn start_chat_only(bind: SocketAddr) -> Result<Self, FixtureError> {
        Self::start_with_responses(bind, false).await
    }

    async fn start_with_responses(
        bind: SocketAddr,
        responses_enabled: bool,
    ) -> Result<Self, FixtureError> {
        let listener = TcpListener::bind(bind).await?;
        let address = listener.local_addr()?;
        let shutdown = CancellationToken::new();
        let shutdown_for_server = shutdown.clone();
        let app = Router::new()
            .route("/v1/chat/completions", post(handlers::chat_completions))
            .route("/v1/responses", post(handlers::responses))
            .with_state(FixtureState { responses_enabled });
        let task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    shutdown_for_server.cancelled().await;
                })
                .await
                .map_err(|error| FixtureError::Serve(error.to_string()))
        });
        Ok(Self {
            address,
            shutdown,
            task,
        })
    }

    /// Returns the base URL to configure in a client, such as `http://127.0.0.1:12345`.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Stops the fixture and waits for its HTTP task to exit.
    pub async fn shutdown(self) -> Result<(), FixtureError> {
        self.shutdown.cancel();
        self.task.await?
    }
}

#[derive(Clone)]
pub(crate) struct FixtureState {
    pub(crate) responses_enabled: bool,
}

pub(crate) const CHAT_COMPLETION_ID: &str = "ahp-test-v1";
pub(crate) const RESPONSES_ID: &str = "ahp-test-response-v1";
pub(crate) const V2_CHAT_COMPLETION_ID: &str = "ahp-test-v2";
pub(crate) const V2_RESPONSES_ID: &str = "ahp-test-response-v2";
