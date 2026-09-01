//! Lifecycle manager for the local fixture and sample-host stack.

mod model_streaming;
mod reports;

use std::path::PathBuf;

use advanced_workspace_host::{AdvancedHost, AdvancedHostConfig, RunningAdvancedHost};
use deterministic_openai::{ObservedRequest, RunningFixture};
use openai_agent_model::ModelConfig;
use uuid::Uuid;

use super::{suite, ConformanceError, Report, RunOptions};

pub(super) use model_streaming::{FixtureSupport, LocalStreamingHost};

pub(super) struct LocalStack {
    artifact: PathBuf,
    workspace_root: PathBuf,
    selected_workspace: PathBuf,
    outside_workspace: PathBuf,
    fixture: RunningFixture,
    config: AdvancedHostConfig,
}

impl LocalStack {
    async fn start(data_dir: PathBuf) -> Result<(Self, RunningAdvancedHost), ConformanceError> {
        std::fs::create_dir_all(&data_dir)?;
        let data_dir = data_dir.canonicalize()?;
        let artifact = data_dir.join(format!("ahp-conformance-{}", Uuid::new_v4()));
        let workspace_root = artifact.join("workspace-root");
        let selected_workspace = workspace_root.join("selected");
        let outside_workspace = artifact.join("outside");
        std::fs::create_dir_all(&selected_workspace)?;
        std::fs::create_dir_all(&outside_workspace)?;
        let bind = "127.0.0.1:0".parse().map_err(|error| {
            ConformanceError::Options(format!("invalid loopback host address: {error}"))
        })?;
        let fixture_bind = "127.0.0.1:0".parse().map_err(|error| {
            ConformanceError::Options(format!("invalid loopback fixture address: {error}"))
        })?;
        let fixture = match RunningFixture::start(fixture_bind).await {
            Ok(fixture) => fixture,
            Err(error) => {
                let fixture_error = ConformanceError::Fixture(error.to_string());
                let cleanup = std::fs::remove_dir_all(&artifact);
                return match cleanup {
                    Ok(()) => Err(fixture_error),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        Err(fixture_error)
                    }
                    Err(cleanup_error) => Err(ConformanceError::Io(cleanup_error)),
                };
            }
        };
        let stack = Self {
            artifact: artifact.clone(),
            workspace_root: workspace_root.clone(),
            selected_workspace,
            outside_workspace,
            config: AdvancedHostConfig {
                bind,
                database_path: artifact.join("database").join("mock-host.redb"),
                model: ModelConfig::new(fixture.base_url(), "fixture-model"),
                discover_models: true,
                workspace_roots: vec![workspace_root],
                reset_shell_approvals: false,
                chunk_size: 11,
                allowed_origins: Vec::new(),
                web_root: None,
            },
            fixture,
        };
        match stack.start_host().await {
            Ok(host) => Ok((stack, host)),
            Err(error) => match stack.cleanup().await {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(cleanup_error),
            },
        }
    }

    pub(super) async fn start_host(&self) -> Result<RunningAdvancedHost, ConformanceError> {
        AdvancedHost::start(self.config.clone())
            .await
            .map_err(|error| ConformanceError::Fixture(error.to_string()))
    }

    pub(super) fn fixture_requests(&self) -> Vec<ObservedRequest> {
        self.fixture.requests()
    }

    async fn cleanup(self) -> Result<(), ConformanceError> {
        let fixture = self
            .fixture
            .shutdown()
            .await
            .map_err(|error| ConformanceError::Fixture(error.to_string()));
        let files = match std::fs::remove_dir_all(&self.artifact) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(ConformanceError::Io(error)),
        };
        match (fixture, files) {
            (Err(error), _) | (_, Err(error)) => Err(error),
            (Ok(()), Ok(())) => Ok(()),
        }
    }
}

pub async fn run_local_suite(
    mut options: RunOptions,
    data_dir: PathBuf,
) -> Result<Report, ConformanceError> {
    suite::validate_selected_cases(&options.selected_cases, true)?;
    let (stack, host) = LocalStack::start(data_dir).await?;
    let target = host.websocket_url();
    options.target = Some(target.clone());
    options.model_checks = true;
    let mut report = suite::run_suite_at(target.clone(), options.clone(), true).await;
    if let Ok(report) = &mut report {
        reports::append_model_config_cases(report, &target, &options, &stack).await;
        reports::append_workspace_tool_cases(report, &target, &options, &stack).await;
    }
    let stopped = host
        .shutdown()
        .await
        .map_err(|error| ConformanceError::Fixture(error.to_string()));
    if let (Ok(report), Ok(())) = (&mut report, &stopped) {
        reports::append_restart_cases(report, &options, &stack).await;
    }
    let cleanup = stack.cleanup().await;
    match (report, stopped, cleanup) {
        (Err(error), _, _) | (_, Err(error), _) | (_, _, Err(error)) => Err(error),
        (Ok(report), Ok(()), Ok(())) => Ok(report),
    }
}
