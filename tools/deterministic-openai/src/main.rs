use std::net::SocketAddr;

use clap::Parser;
use deterministic_openai::RunningFixture;

#[derive(Debug, Parser)]
#[command(about = "Run the deterministic OpenAI-compatible test fixture")]
struct Args {
    /// HTTP bind address.
    #[arg(long, env = "FIXTURE_BIND", default_value = "127.0.0.1:8899")]
    bind: SocketAddr,
    /// Disable `/v1/responses` to exercise automatic Chat Completions fallback.
    #[arg(long)]
    chat_only: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let fixture = if args.chat_only {
        RunningFixture::start_chat_only(args.bind).await?
    } else {
        RunningFixture::start(args.bind).await?
    };
    println!(
        "Deterministic OpenAI fixture listening at {}",
        fixture.base_url()
    );
    tokio::signal::ctrl_c().await?;
    fixture.shutdown().await?;
    Ok(())
}
