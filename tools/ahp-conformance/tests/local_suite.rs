use std::time::Duration;

use ahp_conformance::{run_local_suite, OutputFormat, RunOptions};

#[tokio::test]
async fn deterministic_stack_passes_the_local_suite() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("ahp-conformance-tests");
    let report = run_local_suite(
        RunOptions {
            target: None,
            timeout: Duration::from_secs(5),
            selected_cases: Vec::new(),
            model_checks: true,
            auth_token: None,
            format: OutputFormat::Text,
        },
        data_dir,
    )
    .await?;
    assert!(!report.failed(), "{}", report.render());
    assert_eq!(report.case_count(), 63);
    Ok(())
}
