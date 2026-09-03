use clap::Parser;

use super::{default_database, default_web_root, location_lines, Args};
use crate::WebStatus;

#[test]
fn omitted_production_base_without_a_key_is_rejected() {
    let error = Args::try_parse_from(["advanced-workspace-host"]).expect_err("key is required");
    assert!(error.to_string().contains("--openai-api-key"));
}

#[test]
fn blank_implied_production_key_is_rejected_before_startup(
) -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::try_parse_from(["advanced-workspace-host", "--openai-api-key", "   "])?;
    assert!(args.validate().is_err());
    Ok(())
}

#[test]
fn omitted_base_with_key_uses_production_and_allows_discovery_without_a_model(
) -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::try_parse_from(["advanced-workspace-host", "--openai-api-key", "test-key"])?;
    assert!(args.openai_base_url.is_none());
    assert!(args.openai_model.is_none());
    assert!(!args.no_discover_models);
    let config = args.model_config();
    assert_eq!(config.base_url, "https://api.openai.com");
    assert!(config.response_chaining);
    Ok(())
}

#[test]
fn explicit_base_url_allows_an_optional_key_and_optional_model(
) -> Result<(), Box<dyn std::error::Error>> {
    let custom = Args::try_parse_from([
        "advanced-workspace-host",
        "--openai-base-url",
        "http://127.0.0.1:11434/v1",
    ])?;
    assert_eq!(
        custom.openai_base_url.as_deref(),
        Some("http://127.0.0.1:11434/v1")
    );
    assert!(custom.openai_api_key.is_none());
    assert!(custom.model_config().model.is_none());
    assert!(!custom.model_config().response_chaining);

    let production_override = Args::try_parse_from([
        "advanced-workspace-host",
        "--openai-base-url",
        "https://api.openai.com",
    ])?;
    assert!(production_override.openai_api_key.is_none());
    Ok(())
}

#[test]
fn disabling_discovery_requires_a_model() {
    assert!(Args::try_parse_from([
        "advanced-workspace-host",
        "--openai-base-url",
        "http://127.0.0.1:11434/v1",
        "--no-discover-models",
    ])
    .is_err());
    assert!(Args::try_parse_from([
        "advanced-workspace-host",
        "--openai-base-url",
        "http://127.0.0.1:11434/v1",
        "--no-discover-models",
        "--openai-model",
        "compatible-model",
    ])
    .is_ok());
}

#[test]
fn demo_requires_no_credentials_or_model() {
    assert!(Args::try_parse_from(["advanced-workspace-host", "--demo"]).is_ok());
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
fn default_paths_are_portable() {
    assert_eq!(
        default_database(false),
        std::path::PathBuf::from(".ahp-samples").join("advanced.redb")
    );
    assert_eq!(
        default_database(true),
        std::path::PathBuf::from(".ahp-samples").join("advanced-demo.redb")
    );
    assert_eq!(
        default_web_root(),
        std::path::PathBuf::from("samples")
            .join("web")
            .join("ahp-chat")
            .join("dist")
    );
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
