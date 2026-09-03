use std::path::PathBuf;
use std::time::Duration;

use ahp_conformance::{run_local_suite, run_suite, OutputFormat, RunOptions};
use clap::{Parser, ValueEnum};

mod targets;

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
    #[arg(long, env = "AHP_CONFORMANCE_TARGET", conflicts_with = "local_fixture")]
    target: Option<String>,
    /// Load an authenticated target from the local target store.
    #[arg(long, conflicts_with_all = ["target", "local_fixture"])]
    target_profile: Option<String>,
    /// Save --target, or a hidden interactive URL, under this local profile name.
    #[arg(long, conflicts_with_all = ["target_profile", "local_fixture"])]
    save_target: Option<String>,
    /// Gitignored JSON file containing named authenticated targets.
    #[arg(long, default_value = ".ahp-conformance/targets.json")]
    target_store: PathBuf,
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
    if args.local_fixture
        && (args.target.is_some() || args.target_profile.is_some() || args.save_target.is_some())
    {
        return Err("--local-fixture cannot be combined with an external target".into());
    }
    let prompted_target =
        if args.target.is_none() && args.target_profile.is_none() && args.save_target.is_some() {
            Some(rpassword::prompt_password(
                "AHP WebSocket target (input hidden): ",
            )?)
        } else {
            None
        };
    let supplied_target = args.target.as_ref().or(prompted_target.as_ref());
    let target = match (supplied_target, &args.target_profile) {
        (Some(target), None) => {
            targets::validate_target(target)?;
            if let Some(name) = &args.save_target {
                targets::save(&args.target_store, name, target)?;
                eprintln!(
                    "Saved target profile {name:?} in {}. This file may contain credentials.",
                    args.target_store.display()
                );
            }
            Some(target.clone())
        }
        (None, Some(name)) => Some(targets::load(&args.target_store, name)?),
        (None, None) if args.local_fixture => None,
        (None, None) => return Err("use --target, --target-profile, or --local-fixture".into()),
        (Some(_), Some(_)) => return Err("--target conflicts with --target-profile".into()),
    };
    let options = RunOptions {
        target,
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
