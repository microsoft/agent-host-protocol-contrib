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

fn npm_program() -> &'static str {
    if cfg!(windows) {
        "npm.cmd"
    } else {
        "npm"
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::Path;
    use std::path::PathBuf;

    use deterministic_openai::RunningFixture;
    use openai_agent_model::{ModelApi, ModelConfig};

    use super::{ensure_web, DemoError, NpmRunner};
    use crate::{AdvancedHost, AdvancedHostConfig};

    struct Artifact(PathBuf);

    impl Artifact {
        fn new(name: &str) -> Self {
            Self(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("target")
                    .join(format!("{name}-{}", uuid::Uuid::new_v4())),
            )
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Artifact {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct FakeNpm {
        calls: RefCell<Vec<Vec<String>>>,
        writes_build: bool,
    }

    impl NpmRunner for FakeNpm {
        fn run(&self, directory: &Path, arguments: &[&str]) -> Result<(), DemoError> {
            self.calls
                .borrow_mut()
                .push(arguments.iter().map(ToString::to_string).collect());
            if self.writes_build && arguments == ["run", "build"] {
                std::fs::create_dir_all(directory.join("dist")).map_err(|source| {
                    DemoError::StartNpm {
                        program: "fake-npm",
                        directory: directory.into(),
                        source,
                    }
                })?;
                std::fs::write(directory.join("dist").join("index.html"), "<div>chat</div>")
                    .map_err(|source| DemoError::StartNpm {
                        program: "fake-npm",
                        directory: directory.into(),
                        source,
                    })?;
            }
            Ok(())
        }
    }

    #[test]
    fn prebuilt_web_skips_npm() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new("demo-prebuilt");
        let frontend = artifact.path().join("ahp-chat");
        std::fs::create_dir_all(frontend.join("dist"))?;
        std::fs::write(frontend.join("dist").join("index.html"), "<div>chat</div>")?;
        let runner = FakeNpm {
            calls: RefCell::new(Vec::new()),
            writes_build: false,
        };
        assert_eq!(
            ensure_web(&frontend, false, &runner)?,
            frontend.join("dist")
        );
        assert!(runner.calls.into_inner().is_empty());
        Ok(())
    }

    #[test]
    fn missing_web_installs_then_builds_without_system_npm(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new("demo-build");
        let frontend = artifact.path().join("ahp-chat");
        std::fs::create_dir_all(&frontend)?;
        let runner = FakeNpm {
            calls: RefCell::new(Vec::new()),
            writes_build: true,
        };
        assert_eq!(
            ensure_web(&frontend, false, &runner)?,
            frontend.join("dist")
        );
        assert_eq!(
            runner.calls.into_inner(),
            vec![
                vec!["ci".to_owned()],
                vec!["run".to_owned(), "build".to_owned()]
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn fixture_stops_after_host_and_redb_is_immediately_releasable(
    ) -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new("demo-shutdown");
        let workspace = artifact.path().join("workspace");
        let database = artifact.path().join("demo.redb");
        std::fs::create_dir_all(&workspace)?;
        let fixture = RunningFixture::start("127.0.0.1:0".parse()?).await?;
        let host = AdvancedHost::start(AdvancedHostConfig {
            bind: "127.0.0.1:0".parse()?,
            database_path: database.clone(),
            model: ModelConfig::new(fixture.base_url(), "fixture-model").with_api(ModelApi::Auto),
            workspace_roots: vec![workspace],
            reset_shell_approvals: false,
            chunk_size: 16,
            allowed_origins: Vec::new(),
            web_root: None,
        })
        .await?;

        host.shutdown().await?;
        fixture.shutdown().await?;
        let renamed = artifact.path().join("demo-renamed.redb");
        std::fs::rename(&database, &renamed)?;
        std::fs::remove_file(&renamed)?;
        let reopened = ahp_server_redb::RedbHostStore::open(&database)?;
        reopened.close()?;
        Ok(())
    }
}
