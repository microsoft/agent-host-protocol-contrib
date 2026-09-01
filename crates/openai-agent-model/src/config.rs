//! Configuration and API selection types.

use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use thiserror::Error;

/// Selects which OpenAI-compatible endpoint protocol to use.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ModelApi {
    /// Try Responses first, falling back to Chat Completions only after an
    /// initial Responses HTTP 404, 405, or 501.
    #[default]
    Auto,
    /// Use only the Responses API.
    Responses,
    /// Use only the Chat Completions API.
    ChatCompletions,
}

impl ModelApi {
    /// Returns the canonical configuration value for this API selection.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Responses => "responses",
            Self::ChatCompletions => "chat-completions",
        }
    }
}

impl fmt::Display for ModelApi {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ModelApi {
    type Err = ModelApiParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(Self::Auto),
            "responses" => Ok(Self::Responses),
            "chat-completions" | "chat_completions" | "chatcompletions" => {
                Ok(Self::ChatCompletions)
            }
            _ => Err(ModelApiParseError {
                value: value.to_owned(),
            }),
        }
    }
}

/// An invalid [`ModelApi`] configuration value.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("unknown model API {value:?}; use auto, responses, or chat-completions")]
pub struct ModelApiParseError {
    /// The unrecognized value.
    pub value: String,
}

/// Time limits applied without imposing a total completion deadline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamTimeouts {
    /// Maximum time to establish a connection and receive the initial HTTP
    /// response headers. The default is 10 seconds.
    pub connect: Duration,
    /// Maximum quiet period while waiting for a next response-body chunk. The
    /// default is 75 seconds; every non-empty received body chunk resets it.
    pub idle: Duration,
}

impl Default for StreamTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            idle: Duration::from_secs(75),
        }
    }
}

/// Configuration for an OpenAI-compatible streaming endpoint.
#[derive(Debug, Clone)]
pub struct ModelConfig {
    /// Host URL, `/v1` URL, or complete `/responses` or `/chat/completions`
    /// endpoint URL.
    pub base_url: String,
    /// Model identifier sent in every request.
    pub model: String,
    /// Optional bearer token.
    pub api_key: Option<String>,
    /// Protocol selection. A complete endpoint URL implies its matching API.
    pub api: ModelApi,
    /// Connection/header and per-body-chunk idle timeouts.
    pub timeouts: StreamTimeouts,
}

impl ModelConfig {
    /// Builds a configuration using automatic API selection and default
    /// timeouts.
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            api: ModelApi::Auto,
            api_key: None,
            timeouts: StreamTimeouts::default(),
        }
    }

    /// Adds a bearer token to this configuration.
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Selects an endpoint protocol.
    pub const fn with_api(mut self, api: ModelApi) -> Self {
        self.api = api;
        self
    }

    /// Replaces the stream timeout settings.
    pub const fn with_timeouts(mut self, timeouts: StreamTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
