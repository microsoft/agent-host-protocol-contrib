use clap::Parser;

use super::{location_lines, Args};
use crate::WebStatus;

#[test]
fn demo_does_not_require_openai_options() {
    assert!(Args::try_parse_from(["advanced-workspace-host", "--demo"]).is_ok());
}

#[test]
fn configured_mode_requires_openai_options() {
    assert!(Args::try_parse_from(["advanced-workspace-host"]).is_err());
}

#[test]
fn configured_mode_defaults_to_production_and_accepts_an_endpoint_override(
) -> Result<(), Box<dyn std::error::Error>> {
    let production =
        Args::try_parse_from(["advanced-workspace-host", "--openai-model", "gpt-test"])?;
    assert_eq!(production.openai_base_url, "https://api.openai.com");
    let override_url = Args::try_parse_from([
        "advanced-workspace-host",
        "--openai-model",
        "compatible-model",
        "--openai-base-url",
        "http://127.0.0.1:11434/v1",
    ])?;
    assert_eq!(override_url.openai_base_url, "http://127.0.0.1:11434/v1");
    Ok(())
}

#[test]
fn demo_rejects_incompatible_web_options() {
    assert!(Args::try_parse_from(["advanced-workspace-host", "--demo", "--no-web"]).is_err());
    assert!(Args::try_parse_from([
        "advanced-workspace-host",
        "--demo",
        "--web-root",
        "custom-dist"
    ])
    .is_err());
    assert!(Args::try_parse_from(["advanced-workspace-host", "--rebuild-web"]).is_err());
}

#[test]
fn locations_are_truthful_for_normal_and_demo_hosts() {
    let urls = ("http://127.0.0.1:8787", "ws://127.0.0.1:8787/ahp");
    assert_eq!(
        location_lines(WebStatus::Serving, urls.0, urls.1),
        [
            "AHP WebSocket: ws://127.0.0.1:8787/ahp",
            "Chat UI: http://127.0.0.1:8787/"
        ]
    );
    assert_eq!(
        location_lines(WebStatus::MissingBuild, urls.0, urls.1)[1],
        "Chat UI: unavailable (frontend build missing); setup guidance: http://127.0.0.1:8787/"
    );
    assert_eq!(
        location_lines(WebStatus::Disabled, urls.0, urls.1)[1],
        "Chat UI: unavailable (--no-web); setup guidance: http://127.0.0.1:8787/"
    );
}
