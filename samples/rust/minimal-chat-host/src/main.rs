// Flow: parse command-line settings, then hand the fixed host to its runtime.

mod host;
mod protocol;
mod responses;

use std::net::SocketAddr;

use clap::Parser;
use host::{start, HostConfig};

#[derive(Debug, Parser)]
#[command(about = "A deliberately small AHP host for OpenAI Responses streaming")]
struct Args {
    /// Address for the AHP WebSocket endpoint at /ahp.
    #[arg(long, env = "AHP_BIND", default_value = "127.0.0.1:8787")]
    bind: SocketAddr,

    /// OpenAI API origin, for example https://api.openai.com (without /v1).
    #[arg(long, env = "OPENAI_BASE_URL")]
    openai_base_url: String,

    /// Responses model name.
    #[arg(long, env = "OPENAI_MODEL")]
    openai_model: String,

    /// Optional OpenAI bearer API key.
    #[arg(long, env = "OPENAI_API_KEY")]
    openai_api_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let host = start(HostConfig {
        bind: args.bind,
        openai_base_url: args.openai_base_url,
        openai_model: args.openai_model,
        openai_api_key: args.openai_api_key,
    })
    .await?;

    println!("minimal AHP host listening at {}", host.websocket_url());
    tokio::signal::ctrl_c().await?;
    host.shutdown().await;
    Ok(())
}
