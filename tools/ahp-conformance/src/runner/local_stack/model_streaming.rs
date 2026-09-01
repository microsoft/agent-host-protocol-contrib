//! Isolated fixture/host pairs for model-streaming conformance cases.

use std::path::{Path, PathBuf};

use advanced_workspace_host::{AdvancedHost, AdvancedHostConfig, RunningAdvancedHost};
use deterministic_openai::RunningFixture;
use openai_agent_model::{ModelApi, ModelConfig};
use uuid::Uuid;

use super::super::ConformanceError;
use super::LocalStack;

/// Fixture endpoint support needed by a streaming conformance case.
#[derive(Clone, Copy)]
pub(in super::super) enum FixtureSupport {
    Dual,
    ChatOnly,
}

/// One isolated local model fixture and explicitly configured sample host.
pub(in super::super) struct LocalStreamingHost {
    artifact: PathBuf,
    workspace: PathBuf,
    fixture: RunningFixture,
    host: RunningAdvancedHost,
}

impl LocalStack {
    pub(in super::super) async fn start_streaming_host(
        &self,
        api: ModelApi,
        support: FixtureSupport,
    ) -> Result<LocalStreamingHost, ConformanceError> {
        let fixture_bind = loopback()?;
        let host_bind = loopback()?;
        let artifact = self
            .artifact
            .join(format!("model-streaming-{}", Uuid::new_v4()));
        let workspace_root = artifact.join("workspace-root");
        let workspace = workspace_root.join("selected");
        if let Err(error) = std::fs::create_dir_all(&workspace) {
            let cleanup = remove_artifact(&artifact);
            return match cleanup {
                Ok(()) => Err(ConformanceError::Io(error)),
                Err(error) => Err(error),
            };
        }
        let fixture = match support {
            FixtureSupport::Dual => RunningFixture::start(fixture_bind).await,
            FixtureSupport::ChatOnly => RunningFixture::start_chat_only(fixture_bind).await,
        };
        let fixture = match fixture {
            Ok(fixture) => fixture,
            Err(error) => {
                let files = remove_artifact(&artifact);
                return match files {
                    Ok(()) => Err(ConformanceError::Fixture(error.to_string())),
                    Err(error) => Err(error),
                };
            }
        };
        let config = AdvancedHostConfig {
            bind: host_bind,
            database_path: artifact.join("database").join("mock-host.redb"),
            model: ModelConfig::new(fixture.base_url(), "fixture-model").with_api(api),
            discover_models: false,
            workspace_roots: vec![workspace_root],
            reset_shell_approvals: false,
            chunk_size: 11,
            allowed_origins: Vec::new(),
            web_root: None,
        };
        match AdvancedHost::start(config).await {
            Ok(host) => Ok(LocalStreamingHost {
                artifact,
                workspace,
                fixture,
                host,
            }),
            Err(error) => {
                let fixture_stop = fixture
                    .shutdown()
                    .await
                    .map_err(|stop_error| ConformanceError::Fixture(stop_error.to_string()));
                let files = remove_artifact(&artifact);
                match (fixture_stop, files) {
                    (Err(error), _) | (_, Err(error)) => Err(error),
                    (Ok(()), Ok(())) => Err(ConformanceError::Fixture(error.to_string())),
                }
            }
        }
    }
}

impl LocalStreamingHost {
    pub(in super::super) fn websocket_url(&self) -> String {
        self.host.websocket_url()
    }

    pub(in super::super) fn workspace(&self) -> &Path {
        &self.workspace
    }

    pub(in super::super) async fn shutdown(self) -> Result<(), ConformanceError> {
        let host = self
            .host
            .shutdown()
            .await
            .map_err(|error| ConformanceError::Fixture(error.to_string()));
        let fixture = self
            .fixture
            .shutdown()
            .await
            .map_err(|error| ConformanceError::Fixture(error.to_string()));
        let files = remove_artifact(&self.artifact);
        match (host, fixture, files) {
            (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
            (Ok(()), Ok(()), Ok(())) => Ok(()),
        }
    }
}

fn loopback() -> Result<std::net::SocketAddr, ConformanceError> {
    "127.0.0.1:0"
        .parse()
        .map_err(|error| ConformanceError::Options(format!("invalid loopback address: {error}")))
}

fn remove_artifact(path: &Path) -> Result<(), ConformanceError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ConformanceError::Io(error)),
    }
}
