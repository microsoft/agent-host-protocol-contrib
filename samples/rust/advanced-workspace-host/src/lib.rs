//! Composition root for a persistent, workspace-scoped AHP host.

#![forbid(unsafe_code)]

pub mod cli;
mod web;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ahp_server_core::{CoreConfig, CoreHost, CoreHostError, HostStore, WorkspaceAgentProfile};
use ahp_server_redb::RedbHostStore;
use ahp_server_ws::{router, WsError, WsOptions};
use openai_agent_model::{ModelConfig, ModelError, OpenAiModel};
use thiserror::Error;
use tokio::net::TcpListener;
use tokio::task::JoinError;
use workspace_agent_tools::{ConfiguredWorkspaces, WorkspaceError};

pub use web::WebStatus;

const SERVER_NAME: &str = "advanced-workspace-host";
const OUTBOUND_QUEUE_CAPACITY: usize = 256;

/// Configuration owned by the advanced host composition root.
#[derive(Clone, Debug)]
pub struct AdvancedHostConfig {
    /// TCP address on which the HTTP and WebSocket server listens.
    pub bind: SocketAddr,
    /// Path for durable host state and exact-command approvals.
    pub database_path: PathBuf,
    /// OpenAI-compatible streaming model configuration.
    pub model: ModelConfig,
    /// Existing directories sessions may select as workspaces.
    pub workspace_roots: Vec<PathBuf>,
    /// Clears persisted exact-command approvals before accepting connections.
    pub reset_shell_approvals: bool,
    /// Maximum Unicode scalar values sent in one persisted text delta.
    pub chunk_size: usize,
    /// Additional browser origins permitted for `/ahp` upgrades.
    pub allowed_origins: Vec<String>,
    /// Built Vite directory, or `None` to serve frontend build guidance.
    pub web_root: Option<PathBuf>,
}

/// Errors encountered while constructing or running the composition.
#[derive(Debug, Error)]
pub enum AdvancedHostError {
    /// A listener or server operation failed.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// Workspace roots could not be canonicalized.
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    /// The OpenAI-compatible model configuration was invalid.
    #[error(transparent)]
    Model(#[from] ModelError),
    /// The lifecycle core rejected its configuration or store.
    #[error(transparent)]
    Core(#[from] CoreHostError),
    /// WebSocket origin configuration was invalid.
    #[error(transparent)]
    WebSocket(#[from] WsError),
    /// The durable store could not be opened or reset.
    #[error("redb host store failed: {0}")]
    Store(String),
    /// The HTTP server task did not complete normally.
    #[error("HTTP server task failed: {0}")]
    ServerTask(#[from] JoinError),
}

/// Builder namespace for the advanced host.
pub struct AdvancedHost;

impl AdvancedHost {
    /// Starts the core, WebSocket transport, and optional static frontend.
    pub async fn start(
        config: AdvancedHostConfig,
    ) -> Result<RunningAdvancedHost, AdvancedHostError> {
        let workspaces = if config.workspace_roots.is_empty() {
            ConfiguredWorkspaces::from_current_dir()?
        } else {
            ConfiguredWorkspaces::new(config.workspace_roots)?
        };
        let profile = Arc::new(WorkspaceAgentProfile::new(workspaces));
        create_database_parent(&config.database_path)?;
        let store = Arc::new(
            RedbHostStore::open(&config.database_path)
                .map_err(|error| AdvancedHostError::Store(error.to_string()))?,
        );
        if config.reset_shell_approvals {
            store
                .reset_command_approvals()
                .map_err(|error| AdvancedHostError::Store(error.to_string()))?;
        }
        let model = Arc::new(OpenAiModel::new(config.model)?);
        let core = CoreHost::new(
            CoreConfig {
                server_name: SERVER_NAME.to_owned(),
                max_delta_scalars: config.chunk_size,
                outbound_queue_capacity: OUTBOUND_QUEUE_CAPACITY,
            },
            store.clone(),
            model,
            profile,
        )?;
        let web = web::compose(config.web_root);
        let app = router(
            core.clone(),
            WsOptions {
                allowed_origins: config.allowed_origins,
            },
        )?
        .merge(web.router);
        let listener = TcpListener::bind(config.bind).await?;
        let address = listener.local_addr()?;
        let shutdown = core.shutdown();
        let server_shutdown = shutdown.clone();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    server_shutdown.cancelled().await;
                })
                .await
        });

        Ok(RunningAdvancedHost {
            address,
            core,
            store,
            shutdown,
            server: Some(server),
            web_status: web.status,
        })
    }
}

/// A started advanced host and its graceful-shutdown handle.
pub struct RunningAdvancedHost {
    address: SocketAddr,
    core: CoreHost,
    store: Arc<RedbHostStore>,
    shutdown: ahp_server_core::Shutdown,
    server: Option<tokio::task::JoinHandle<Result<(), std::io::Error>>>,
    web_status: WebStatus,
}

impl RunningAdvancedHost {
    /// Returns the HTTP origin serving the frontend and `/ahp`.
    #[must_use]
    pub fn http_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Returns the AHP WebSocket endpoint URL.
    #[must_use]
    pub fn websocket_url(&self) -> String {
        format!("ws://{}/ahp", self.address)
    }

    /// Returns whether the root HTTP route serves the browser UI.
    #[must_use]
    pub const fn web_status(&self) -> WebStatus {
        self.web_status
    }

    /// Stops the transport, drains active turns, and releases the redb file handle.
    pub async fn shutdown(mut self) -> Result<(), AdvancedHostError> {
        self.shutdown.cancel();
        let server = match self.server.take() {
            Some(server) => match server.await {
                Ok(result) => result.map_err(AdvancedHostError::from),
                Err(error) => Err(AdvancedHostError::ServerTask(error)),
            },
            None => Ok(()),
        };
        let core = self.core.shutdown_and_wait().await;
        let store = self
            .store
            .close()
            .map_err(|error| AdvancedHostError::Store(error.to_string()));
        server?;
        core?;
        store?;
        Ok(())
    }
}

impl Drop for RunningAdvancedHost {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn create_database_parent(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
