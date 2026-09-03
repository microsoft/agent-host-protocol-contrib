//! One-command deterministic browser demo orchestration.

use std::path::{Path, PathBuf};
use std::process::Command;

use deterministic_openai::RunningFixture;
use thiserror::Error;

use super::{run_host, Args};
use crate::AdvancedHost;

#[derive(Debug, Error)]
enum DemoError {
    #[error("could not start {program} in {directory:?}: {source}")]
    StartNpm {
        program: &'static str,
        directory: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{program} {arguments:?} failed with {status} in {directory:?}")]
    NpmFailed {
        program: &'static str,
        arguments: Vec<String>,
        status: std::process::ExitStatus,
        directory: PathBuf,
    },
    #[error("frontend build did not produce a non-empty {0:?}")]
    MissingBuild(PathBuf),
}

trait NpmRunner {
    fn run(&self, directory: &Path, arguments: &[&str]) -> Result<(), DemoError>;
}

struct SystemNpm;

impl NpmRunner for SystemNpm {
    fn run(&self, directory: &Path, arguments: &[&str]) -> Result<(), DemoError> {
        let program = npm_program();
        let status = Command::new(program)
            .args(arguments)
            .current_dir(directory)
            .status()
            .map_err(|source| DemoError::StartNpm {
                program,
                directory: directory.into(),
                source,
            })?;
        if status.success() {
            return Ok(());
        }
        Err(DemoError::NpmFailed {
            program,
            arguments: arguments.iter().map(ToString::to_string).collect(),
            status,
            directory: directory.into(),
        })
    }
}

/// Starts the internal fixture and advanced host for `--demo`.
pub(super) async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let web_root = ensure_web(&frontend_directory(), args.rebuild_web, &SystemNpm)?;
    let fixture = RunningFixture::start("127.0.0.1:0".parse()?).await?;
    let model = args.fixture_model_config(fixture.base_url());
    let host = match AdvancedHost::start(args.host_config(model, Some(web_root))).await {
        Ok(host) => host,
        Err(error) => {
            fixture.shutdown().await?;
            return Err(error.into());
        }
    };
    let result = run_host(
        host,
        Some("Deterministic OpenAI fixture: internal loopback server."),
    )
    .await;
    let fixture_result = fixture.shutdown().await;
    result?;
    fixture_result?;
    Ok(())
}

fn frontend_directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("web")
        .join("ahp-chat")
}

fn ensure_web(
    frontend: &Path,
    rebuild: bool,
    runner: &impl NpmRunner,
) -> Result<PathBuf, DemoError> {
    let dist = frontend.join("dist");
    if !rebuild && crate::web::is_built(&dist) {
        return Ok(dist);
    }
    if needs_install(frontend) {
        runner.run(frontend, &["ci"])?;
    }
    runner.run(frontend, &["run", "build"])?;
    if crate::web::is_built(&dist) {
        Ok(dist)
    } else {
        Err(DemoError::MissingBuild(dist.join("index.html")))
    }
}

fn needs_install(frontend: &Path) -> bool {
    let node_modules = frontend.join("node_modules");
    if !node_modules.is_dir() {
        return true;
    }
    let package_lock = frontend.join("package-lock.json");
    let installed_lock = node_modules.join(".package-lock.json");
    match (package_lock.metadata(), installed_lock.metadata()) {
        (Ok(package_lock), Ok(installed_lock)) => {
            matches!(
                (package_lock.modified(), installed_lock.modified()),
                (Ok(package_lock), Ok(installed_lock)) if package_lock > installed_lock
            )
        }
        (Ok(_), Err(_)) => true,
        _ => false,
    }
}

#[cfg(windows)]
fn npm_program() -> &'static str {
    "npm.cmd"
}

#[cfg(not(windows))]
fn npm_program() -> &'static str {
    "npm"
}

#[cfg(test)]
#[path = "demo_tests.rs"]
mod tests;
