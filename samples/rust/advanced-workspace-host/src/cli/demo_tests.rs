use std::cell::RefCell;
use std::path::{Path, PathBuf};

use clap::Parser;
use deterministic_openai::RunningFixture;
use openai_agent_model::{ModelApi, ModelConfig};

use super::{ensure_web, DemoError, NpmRunner};
use crate::cli::Args;
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
            std::fs::write(directory.join("dist").join("index.html"), "<div>chat</div>").map_err(
                |source| DemoError::StartNpm {
                    program: "fake-npm",
                    directory: directory.into(),
                    source,
                },
            )?;
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
fn missing_web_installs_then_builds_without_system_npm() -> Result<(), Box<dyn std::error::Error>> {
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
        model: ModelConfig::new(fixture.base_url())
            .with_default_model("fixture-model")
            .with_api(ModelApi::Auto),
        discover_models: true,
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

#[tokio::test]
async fn demo_fixture_discovers_multiple_model_choices() -> Result<(), Box<dyn std::error::Error>> {
    let artifact = Artifact::new("demo-model-catalog");
    let workspace = artifact.path().join("workspace");
    std::fs::create_dir_all(&workspace)?;
    let fixture = RunningFixture::start("127.0.0.1:0".parse()?).await?;
    let result: Result<(), Box<dyn std::error::Error>> = async {
        let args = Args::try_parse_from(["advanced-workspace-host", "--demo"])?;
        let host = AdvancedHost::start(AdvancedHostConfig {
            bind: "127.0.0.1:0".parse()?,
            database_path: artifact.path().join("demo.redb"),
            model: args.fixture_model_config(fixture.base_url()),
            discover_models: true,
            workspace_roots: vec![workspace],
            reset_shell_approvals: false,
            chunk_size: 16,
            allowed_origins: Vec::new(),
            web_root: None,
        })
        .await?;
        assert_eq!(
            host.model_summary(),
            "Models: 3 available; default fixture-model"
        );
        host.shutdown().await?;
        Ok(())
    }
    .await;
    let fixture_result = fixture.shutdown().await;
    result?;
    fixture_result?;
    Ok(())
}
