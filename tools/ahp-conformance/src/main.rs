use std::path::PathBuf;
use std::time::Duration;

use ahp_conformance::{run_local_suite, run_suite, OutputFormat, RunOptions};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, ValueEnum)]
enum Format {
    Text,
    Json,
}

impl From<Format> for OutputFormat {
    fn from(value: Format) -> Self {
        match value {
            Format::Text => Self::Text,
            Format::Json => Self::Json,
        }
    }
}

#[derive(Debug, Parser)]
#[command(about = "Run observable AHP conformance probes")]
struct Args {
    /// AHP WebSocket URL to probe.
    #[arg(long, required_unless_present = "local_fixture")]
    target: Option<String>,
    /// Start the deterministic model and mock host on ephemeral ports. This also
    /// enables the local-fixture-only persistence probe.
    #[arg(long)]
    local_fixture: bool,
    /// Directory under the current workspace used only for the local fixture DB.
    #[arg(long, default_value = "target/ahp-conformance")]
    local_data_dir: PathBuf,
    /// Per-operation timeout in milliseconds.
    #[arg(long, default_value_t = 5_000)]
    timeout_ms: u64,
    /// Run only these named cases; may be passed more than once.
    #[arg(long)]
    case: Vec<String>,
    /// Enable exact content checks using the documented deterministic fixture
    /// prompt syntax. Leave off for a real, nondeterministic model.
    #[arg(long)]
    model_checks: bool,
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let options = RunOptions {
        target: args.target,
        timeout: Duration::from_millis(args.timeout_ms),
        selected_cases: args.case,
        model_checks: args.model_checks,
        format: args.format.into(),
    };
    let report = if args.local_fixture {
        run_local_suite(options, args.local_data_dir).await?
    } else {
        run_suite(options).await?
    };
    print!("{}", report.render());
    if report.failed() {
        std::process::exit(1);
    }
    Ok(())
}
