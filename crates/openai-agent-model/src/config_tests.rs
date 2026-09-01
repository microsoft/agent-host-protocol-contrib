use std::str::FromStr;
use std::time::Duration;

use crate::{ModelApi, ModelApiParseError, ModelConfig, StreamTimeouts};

#[test]
fn parses_and_displays_api_selection() -> Result<(), ModelApiParseError> {
    assert_eq!(ModelApi::default(), ModelApi::Auto);
    assert_eq!(ModelApi::from_str("RESPONSES")?, ModelApi::Responses);
    assert_eq!(
        ModelApi::from_str("chat_completions")?,
        ModelApi::ChatCompletions
    );
    assert_eq!(ModelApi::ChatCompletions.to_string(), "chat-completions");
    assert!(ModelApi::from_str("other").is_err());
    Ok(())
}

#[test]
fn builder_keeps_public_field_defaults() {
    let timeouts = StreamTimeouts {
        connect: Duration::from_secs(2),
        idle: Duration::from_secs(3),
    };
    let config = ModelConfig::new("https://example.test", "model")
        .with_api_key("secret")
        .with_api(ModelApi::Responses)
        .with_timeouts(timeouts);
    assert_eq!(config.api_key.as_deref(), Some("secret"));
    assert_eq!(config.api, ModelApi::Responses);
    assert_eq!(config.timeouts, timeouts);
    assert_eq!(StreamTimeouts::default().connect, Duration::from_secs(10));
    assert_eq!(StreamTimeouts::default().idle, Duration::from_secs(75));
}
