//! Command-line configuration for configured and one-command demo hosting.

mod demo;

use std::net::SocketAddr;
use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use openai_agent_model::{ModelApi, ModelConfig, StreamTimeouts};

use crate::{AdvancedHost, AdvancedHostConfig, RunningAdvancedHost, WebStatus};

#[derive(Debug, Parser)]
#[command(about = "Run the advanced persistent workspace AHP host")]
pub(super) struct Args {
    /// HTTP/WebSocket bind address.
    #[arg(long, env = "AHP_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,
    /// redb database path. Defaults to the demo-specific database with --demo.
    #[arg(long, env = "AHP_DATABASE")]
    database: Option<PathBuf>,
    /// OpenAI-compatible host, /v1 path, or complete endpoint. Defaults to api.openai.com.
    #[arg(
        long,
        env = "OPENAI_BASE_URL",
        default_value = "https://api.openai.com"
    )]
    openai_base_url: String,
    /// Model identifier sent with each streamed completion request.
    #[arg(long, env = "OPENAI_MODEL", required_unless_present = "demo")]
    openai_model: Option<String>,
    /// Use only `--openai-model` rather than querying the endpoint's `/v1/models` catalog.
    #[arg(long, env = "AHP_NO_DISCOVER_MODELS", conflicts_with = "demo")]
    no_discover_models: bool,
    /// Optional bearer API key.
    #[arg(long, env = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
    /// Endpoint protocol; auto falls back only after initial Responses 404/405/501.
    #[arg(long, env = "OPENAI_API", value_enum, default_value_t = OpenAiApi::Auto)]
    openai_api: OpenAiApi,
    /// Start an internal deterministic fixture and build/serve the chat UI.
    #[arg(long, conflicts_with = "no_web")]
    demo: bool,
    /// Rebuild the React UI before starting the demo.
    #[arg(long, requires = "demo")]
    rebuild_web: bool,
    /// Maximum Unicode scalar values in one AHP chat delta.
    #[arg(long, env = "AHP_CHUNK_SIZE", default_value_t = 16)]
    chunk_size: usize,
    /// Connection and initial-response-header timeout in seconds.
    #[arg(long, env = "OPENAI_CONNECT_TIMEOUT_SECONDS", default_value = "10")]
    connect_timeout_seconds: NonZeroU64,
    /// Quiet interval between streamed chunks in seconds; there is no total stream deadline.
    #[arg(long, env = "OPENAI_IDLE_TIMEOUT_SECONDS", default_value = "75")]
    idle_timeout_seconds: NonZeroU64,
    /// Additional browser Origin allowed to upgrade `/ahp`; repeat or comma-separate values.
    #[arg(long, env = "AHP_ALLOWED_ORIGIN", value_delimiter = ',')]
    allowed_origin: Vec<String>,
    /// Allowed session workspace root; repeat or comma-separate values. Defaults to the current directory.
    #[arg(long, env = "AHP_WORKSPACE_ROOT", value_delimiter = ',')]
    workspace_root: Vec<PathBuf>,
    /// Clear persisted exact, trimmed shell-command approvals; not an executable allowlist.
    #[arg(long, env = "AHP_RESET_SHELL_APPROVALS")]
    reset_shell_approvals: bool,
    /// Built Vite frontend directory served at `/`.
    #[arg(
        long,
        env = "AHP_WEB_ROOT",
        default_value_os_t = default_web_root(),
        conflicts_with = "demo"
    )]
    web_root: PathBuf,
    /// Serve frontend build guidance at `/` instead of static files.
    #[arg(long, env = "AHP_NO_WEB", conflicts_with = "web_root")]
    no_web: bool,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OpenAiApi {
    Auto,
    Responses,
    ChatCompletions,
}

impl From<OpenAiApi> for ModelApi {
    fn from(value: OpenAiApi) -> Self {
        match value {
            OpenAiApi::Auto => Self::Auto,
            OpenAiApi::Responses => Self::Responses,
            OpenAiApi::ChatCompletions => Self::ChatCompletions,
        }
    }
}

impl Args {
    fn model_config(&self) -> Result<ModelConfig, std::io::Error> {
        let model_name = self.openai_model.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "--openai-model is required unless --demo is used",
            )
        })?;
        Ok(self.apply_model_options(ModelConfig::new(self.openai_base_url.clone(), model_name)))
    }

    pub(super) fn fixture_model_config(&self, base_url: String) -> ModelConfig {
        ModelConfig::new(base_url, "fixture-model")
            .with_api(ModelApi::Auto)
            .with_timeouts(StreamTimeouts {
                connect: Duration::from_secs(self.connect_timeout_seconds.get()),
                idle: Duration::from_secs(self.idle_timeout_seconds.get()),
            })
    }

    fn apply_model_options(&self, model: ModelConfig) -> ModelConfig {
        let model = model
            .with_api(self.openai_api.into())
            .with_timeouts(StreamTimeouts {
                connect: Duration::from_secs(self.connect_timeout_seconds.get()),
                idle: Duration::from_secs(self.idle_timeout_seconds.get()),
            });
        match self.openai_api_key.clone() {
            Some(key) => model.with_api_key(key),
            None => model,
        }
    }

    pub(super) fn host_config(
        self,
        model: ModelConfig,
        web_root: Option<PathBuf>,
    ) -> AdvancedHostConfig {
        let demo = self.demo;
        AdvancedHostConfig {
            bind: self.bind,
            database_path: self.database.unwrap_or_else(|| default_database(demo)),
            model,
            discover_models: !self.no_discover_models,
            workspace_roots: self.workspace_root,
            reset_shell_approvals: self.reset_shell_approvals,
            chunk_size: self.chunk_size,
            allowed_origins: self.allowed_origin,
            web_root,
        }
    }
}

/// Parses the standard host CLI, starts the selected host, and waits for Ctrl-C.
pub async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.demo {
        return demo::run(args).await;
    }
    let model = args.model_config()?;
    let web_root = (!args.no_web).then_some(args.web_root.clone());
    let host = AdvancedHost::start(args.host_config(model, web_root)).await?;
    run_host(host, None).await
}

pub(super) async fn run_host(
    host: RunningAdvancedHost,
    fixture_note: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    for line in location_lines(host.web_status(), &host.http_url(), &host.websocket_url()) {
        println!("{line}");
    }
    println!("{}", host.model_summary());
    if let Some(note) = fixture_note {
        println!("{note}");
    }
    println!("Open the Chat UI and click Connect to begin.");
    let signal = tokio::signal::ctrl_c().await;
    let stopped = host.shutdown().await;
    signal?;
    stopped?;
    Ok(())
}

pub(super) fn location_lines(
    status: WebStatus,
    http_url: &str,
    websocket_url: &str,
) -> [String; 2] {
    let setup_url = format!("{http_url}/");
    let chat = match status {
        WebStatus::Serving => format!("Chat UI: {setup_url}"),
        WebStatus::Disabled => {
            format!("Chat UI: unavailable (--no-web); setup guidance: {setup_url}")
        }
        WebStatus::MissingBuild => {
            format!("Chat UI: unavailable (frontend build missing); setup guidance: {setup_url}")
        }
    };
    [format!("AHP WebSocket: {websocket_url}"), chat]
}

fn default_database(demo: bool) -> PathBuf {
    if demo {
        PathBuf::from(".ahp-samples\\advanced-demo.redb")
    } else {
        PathBuf::from(".ahp-samples\\advanced.redb")
    }
}

fn default_web_root() -> PathBuf {
    PathBuf::from("samples\\web\\ahp-chat\\dist")
}

#[cfg(test)]
mod tests;
