// Flow: parse command-line settings, then hand the fixed host to its runtime.

mod host;
mod protocol;
mod responses;

use std::net::SocketAddr;

use clap::Parser;
use host::{start, HostConfig};

const PRODUCTION_OPENAI_BASE_URL: &str = "https://api.openai.com";
const DEFAULT_OPENAI_MODEL: &str = "gpt-5.6-luna";

#[derive(Debug, Parser)]
#[command(about = "A deliberately small AHP host for OpenAI Responses streaming")]
struct Args {
    /// Address for the AHP WebSocket endpoint at /ahp.
    #[arg(long, env = "AHP_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,

    /// OpenAI API origin without /v1; omit to use api.openai.com.
    #[arg(long, env = "OPENAI_BASE_URL")]
    openai_base_url: Option<String>,

    /// Responses model name.
    #[arg(long, env = "OPENAI_MODEL", default_value = DEFAULT_OPENAI_MODEL)]
    openai_model: String,

    /// Bearer key; required when the production base URL is implied.
    #[arg(
        long,
        env = "OPENAI_API_KEY",
        required_unless_present = "openai_base_url"
    )]
    openai_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let production_base_implied = args.openai_base_url.is_none();
    if production_base_implied
        && args
            .openai_api_key
            .as_deref()
            .is_none_or(|key| key.trim().is_empty())
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "an OpenAI API key is required when --openai-base-url is omitted",
        )
        .into());
    }
    let host = start(HostConfig {
        bind: args.bind,
        openai_base_url: args
            .openai_base_url
            .unwrap_or_else(|| PRODUCTION_OPENAI_BASE_URL.to_owned()),
        openai_model: args.openai_model,
        openai_api_key: args.openai_api_key,
        response_chaining: production_base_implied,
    })
    .await?;

    println!("minimal AHP host listening at {}", host.websocket_url());
    tokio::signal::ctrl_c().await?;
    host.shutdown().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{Args, DEFAULT_OPENAI_MODEL, PRODUCTION_OPENAI_BASE_URL};

    #[test]
    fn production_defaults_need_only_an_api_key() {
        let args = Args::try_parse_from(["minimal-chat-host", "--openai-api-key", "test-key"])
            .expect("API key alone should select production defaults");
        assert!(args.openai_base_url.is_none());
        assert_eq!(args.openai_model, DEFAULT_OPENAI_MODEL);
        assert_eq!(PRODUCTION_OPENAI_BASE_URL, "https://api.openai.com");
    }

    #[test]
    fn an_explicit_compatible_endpoint_does_not_require_a_key() {
        let args = Args::try_parse_from([
            "minimal-chat-host",
            "--openai-base-url",
            "http://127.0.0.1:8899",
            "--openai-model",
            "fixture-model",
        ])
        .expect("explicit compatible endpoints may omit credentials");
        assert_eq!(
            args.openai_base_url.as_deref(),
            Some("http://127.0.0.1:8899")
        );
        assert_eq!(args.openai_model, "fixture-model");
    }

    #[test]
    fn omitted_base_without_a_key_is_rejected() {
        assert!(Args::try_parse_from(["minimal-chat-host"]).is_err());
    }
}
