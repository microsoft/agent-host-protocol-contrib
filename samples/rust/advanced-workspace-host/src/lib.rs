//! Composition root for a persistent, workspace-scoped AHP host.

#![forbid(unsafe_code)]

pub mod cli;
mod web;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ahp_server_core::{
    CoreConfig, CoreHost, CoreHostError, HostStore, ModelCatalog, WorkspaceAgentProfile,
};
use ahp_server_redb::RedbHostStore;
use ahp_server_ws::{router, WsError, WsOptions};
use openai_agent_model::{ModelConfig, ModelDescriptor, ModelError, OpenAiModel};
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
    /// Whether to query the endpoint's `/v1/models` catalog before serving.
    pub discover_models: bool,
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
        let default_model = config.model.model.clone();
        let model = Arc::new(OpenAiModel::new(config.model)?);
        let model_catalog =
            discover_model_catalog(&model, &default_model, config.discover_models).await?;
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
        let model_count = model_catalog.models.len();
        let core = CoreHost::with_model_catalog(
            CoreConfig {
                server_name: SERVER_NAME.to_owned(),
                max_delta_scalars: config.chunk_size,
                outbound_queue_capacity: OUTBOUND_QUEUE_CAPACITY,
            },
            model_catalog,
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
            model_count,
            default_model,
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
    model_count: usize,
    default_model: String,
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

    /// Returns the discovered model count and configured default model identifier.
    #[must_use]
    pub fn model_summary(&self) -> String {
        format!(
            "Models: {} available; default {}",
            self.model_count, self.default_model
        )
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

async fn discover_model_catalog(
    model: &OpenAiModel,
    default_model: &str,
    discover_models: bool,
) -> Result<ModelCatalog, AdvancedHostError> {
    let descriptors = if discover_models {
        match model.list_models().await {
            Ok(descriptors) => descriptors,
            Err(ModelError::ModelsUnsupported { status, .. }) => {
                eprintln!(
                    "Model discovery is unavailable (HTTP {status}); using only configured default {default_model}."
                );
                Vec::new()
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        Vec::new()
    };
    model_catalog(default_model, descriptors)
}

fn model_catalog(
    default_model: &str,
    mut descriptors: Vec<ModelDescriptor>,
) -> Result<ModelCatalog, AdvancedHostError> {
    descriptors.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.owned_by.cmp(&right.owned_by))
    });
    descriptors.dedup_by(|left, right| left.id == right.id);
    if !descriptors
        .iter()
        .any(|descriptor| descriptor.id == default_model)
    {
        descriptors.insert(
            0,
            ModelDescriptor {
                id: default_model.to_owned(),
                owned_by: None,
            },
        );
    }

    Ok(ModelCatalog::from_model_descriptors_with_thinking_levels(
        descriptors,
        default_model,
        |descriptor| thinking_levels_for(&descriptor.id).iter().copied(),
    )
    .map_err(CoreHostError::from)?)
}

fn thinking_levels_for(model_id: &str) -> &'static [&'static str] {
    const FAST: &[&str] = &["none", "minimal", "low"];
    const GENERAL: &[&str] = &["minimal", "low", "medium", "high"];
    const ADVANCED: &[&str] = &["low", "medium", "high", "xhigh"];
    const EXTENDED: &[&str] = &["low", "medium", "high", "xhigh", "max"];
    const GPT_5_1_OR_5_2: &[&str] = &["none", "minimal", "low", "medium", "high"];

    let model_id = model_id.to_ascii_lowercase();
    if model_id.contains("fixture-fast") || model_id.contains("mini") || model_id.contains("nano") {
        FAST
    } else if model_id.contains("fixture-reasoning")
        || model_id.starts_with("gpt-5.6")
        || model_id.contains("max")
    {
        EXTENDED
    } else if model_id.starts_with("gpt-5.5")
        || model_id.starts_with("gpt-5.4")
        || model_id.starts_with("gpt-5.3")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
    {
        ADVANCED
    } else if model_id.starts_with("gpt-5.1") || model_id.starts_with("gpt-5.2") {
        GPT_5_1_OR_5_2
    } else {
        GENERAL
    }
}

#[cfg(test)]
mod tests;
