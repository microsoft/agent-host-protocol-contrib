//! Public requests, events, replies, and errors.

use std::fmt;
use std::pin::Pin;
use std::str::FromStr;
use std::time::Duration;

use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

/// A model advertised by an OpenAI-compatible model catalog.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelDescriptor {
    /// Provider model identifier.
    pub id: String,
    /// Provider account or organization that owns the model, when advertised.
    pub owned_by: Option<String>,
}

/// Selects the model and optional reasoning effort for one request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSelection {
    /// Model identifier sent for this request.
    pub id: String,
    /// Reasoning effort sent only when explicitly selected.
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl ModelSelection {
    /// Creates a selection with no reasoning-effort override.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            reasoning_effort: None,
        }
    }

    /// Adds a reasoning-effort override.
    #[must_use]
    pub const fn with_reasoning_effort(mut self, reasoning_effort: ReasoningEffort) -> Self {
        self.reasoning_effort = Some(reasoning_effort);
        self
    }
}

/// Model-dependent reasoning effort requested for a completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Disable reasoning tokens when the selected model supports that setting.
    None,
    /// Request minimal reasoning.
    Minimal,
    /// Request low reasoning.
    Low,
    /// Request medium reasoning.
    Medium,
    /// Request high reasoning.
    High,
    /// Request extra-high reasoning.
    XHigh,
    /// Request maximum reasoning.
    Max,
}

impl ReasoningEffort {
    /// Returns the OpenAI wire value.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for ReasoningEffort {
    type Err = ReasoningEffortParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" => Ok(Self::None),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            "max" => Ok(Self::Max),
            _ => Err(ReasoningEffortParseError {
                value: value.to_owned(),
            }),
        }
    }
}

/// An unsupported reasoning-effort value.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("unknown reasoning effort {value:?}; use none, minimal, low, medium, high, xhigh, or max")]
pub struct ReasoningEffortParseError {
    /// The unrecognized value.
    pub value: String,
}

/// One model completion request, including conversation state and tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRequest {
    /// Ordered conversation items.
    pub messages: Vec<ModelMessage>,
    /// Function tools that the model may call.
    pub tools: Vec<ModelToolDefinition>,
    /// Optional model and reasoning-effort override for this request.
    pub model_selection: Option<ModelSelection>,
    /// Optional Responses API chain cursor and the first message not represented by it.
    pub response_continuation: Option<ResponseContinuation>,
    /// Whether the Responses API should retain this response for continuation.
    pub store_response: bool,
}

impl ModelRequest {
    /// Creates a text-only request with one user message.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            messages: vec![ModelMessage::User {
                content: content.into(),
            }],
            tools: Vec::new(),
            model_selection: None,
            response_continuation: None,
            store_response: false,
        }
    }

    /// Adds a per-request model selection.
    #[must_use]
    pub fn with_model_selection(mut self, model_selection: ModelSelection) -> Self {
        self.model_selection = Some(model_selection);
        self
    }

    /// Continues a stored Responses API chain while retaining full messages for
    /// a possible Chat Completions fallback.
    #[must_use]
    pub fn with_response_continuation(
        mut self,
        previous_response_id: impl Into<String>,
        input_start: usize,
    ) -> Self {
        self.response_continuation = Some(ResponseContinuation {
            previous_response_id: previous_response_id.into(),
            input_start,
        });
        self
    }

    /// Requests provider-side storage for Responses API continuation.
    #[must_use]
    pub const fn with_response_storage(mut self, store: bool) -> Self {
        self.store_response = store;
        self
    }

    pub(crate) fn selected_model<'a>(
        &'a self,
        configured_model: Option<&'a str>,
    ) -> Result<(&'a str, Option<ReasoningEffort>), ModelError> {
        match &self.model_selection {
            Some(selection) if selection.id.trim().is_empty() => {
                Err(ModelError::InvalidModelSelection {
                    reason: "model identifier must not be empty".to_owned(),
                })
            }
            Some(selection) => Ok((&selection.id, selection.reasoning_effort)),
            None => configured_model
                .filter(|model| !model.trim().is_empty())
                .map(|model| (model, None))
                .ok_or(ModelError::MissingModel),
        }
    }
}

/// Responses API continuation state for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseContinuation {
    /// ID returned by the previous stored OpenAI response.
    pub previous_response_id: String,
    /// Index of the first message that is not already represented by the cursor.
    pub input_start: usize,
}

/// A conversation item understood by both supported OpenAI APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelMessage {
    /// A user text message.
    User {
        /// User-provided text.
        content: String,
    },
    /// An assistant message, optionally with previous tool calls.
    Assistant {
        /// Assistant text, if any.
        content: Option<String>,
        /// Tool calls emitted by the assistant.
        tool_calls: Vec<ModelToolCall>,
    },
    /// The result of a previously requested function call.
    Tool {
        /// The corresponding model call ID.
        tool_call_id: String,
        /// Tool output passed back to the model verbatim.
        content: String,
    },
}

/// A function tool exposed to a model.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelToolDefinition {
    /// Stable function name.
    pub name: String,
    /// Human-readable purpose.
    pub description: String,
    /// JSON Schema describing the function input.
    pub input_schema: Value,
}

impl ModelToolDefinition {
    /// Creates a function tool definition.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// A function call returned by the model.
///
/// `arguments` remains the raw JSON string so consumers control validation and
/// parsing before invoking a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelToolCall {
    /// Provider call identifier.
    pub id: String,
    /// Function name.
    pub name: String,
    /// Raw function argument JSON.
    pub arguments: String,
}

/// Fully collected text and tool calls from one assistant completion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelReply {
    /// Assistant text, or `None` when the reply contains only tool calls.
    pub content: Option<String>,
    /// Function calls requested by the model.
    pub tool_calls: Vec<ModelToolCall>,
    /// Responses API identifier, when the selected endpoint supplied one.
    pub response_id: Option<String>,
}

/// One decoded incremental item from a model stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStreamEvent {
    /// A Responses API stream identified the response being created.
    ResponseStarted {
        /// Provider response identifier.
        id: String,
    },
    /// A piece of assistant text.
    TextDelta(String),
    /// A function call whose identity is now known.
    ToolCallStarted {
        /// API-provided ordinal for this call.
        index: usize,
        /// Provider call identifier.
        id: String,
        /// Function name.
        name: String,
    },
    /// A piece of raw JSON function arguments.
    ToolCallArgumentsDelta {
        /// API-provided ordinal for the call.
        index: usize,
        /// Raw JSON fragment.
        delta: String,
    },
    /// The protocol-specific terminal event was validated.
    Completed,
}

/// A cancel-on-drop stream of decoded model events.
pub type ModelEventStream =
    Pin<Box<dyn Stream<Item = Result<ModelStreamEvent, ModelError>> + Send>>;

/// Failures returned by an OpenAI-compatible endpoint or stream.
#[derive(Debug, Error)]
pub enum ModelError {
    /// Neither this request nor the client configuration selected a model.
    #[error(
        "no model was selected; configure a default model or set ModelRequest.model_selection"
    )]
    MissingModel,
    /// A configured endpoint cannot form a valid OpenAI endpoint URL.
    #[error("invalid OpenAI base URL {base_url:?}: {reason}")]
    InvalidBaseUrl {
        /// Configured URL.
        base_url: String,
        /// Validation failure.
        reason: String,
    },
    /// A required timeout was zero.
    #[error("{name} timeout must be greater than zero")]
    InvalidTimeout {
        /// Name of the invalid timeout setting.
        name: &'static str,
    },
    /// A per-request model selection was invalid before HTTP dispatch.
    #[error("invalid model selection: {reason}")]
    InvalidModelSelection {
        /// Validation failure.
        reason: String,
    },
    /// Connecting or waiting for the initial HTTP response headers took too
    /// long. This is not a total request timeout.
    #[error("OpenAI initial response timed out after {timeout:?}")]
    ResponseTimeout {
        /// Configured connection/header timeout.
        timeout: Duration,
    },
    /// The upstream stream produced no non-empty response-body chunk before
    /// its configured idle deadline.
    #[error("OpenAI stream was idle for {timeout:?}")]
    IdleTimeout {
        /// Configured per-chunk idle timeout.
        timeout: Duration,
    },
    /// The HTTP request or response body failed.
    #[error("OpenAI request failed: {0}")]
    Request(#[from] reqwest::Error),
    /// The endpoint returned a non-success HTTP status.
    #[error("OpenAI endpoint returned HTTP {status}: {body}")]
    Upstream {
        /// Returned status code.
        status: reqwest::StatusCode,
        /// Bounded text captured from the returned error body.
        body: String,
    },
    /// Model discovery is not supported by the configured endpoint.
    #[error("OpenAI model discovery is unsupported (HTTP {status}): {body}")]
    ModelsUnsupported {
        /// Returned status code.
        status: reqwest::StatusCode,
        /// Bounded text captured from the returned error body.
        body: String,
    },
    /// A streamed API event reported a model failure.
    #[error("OpenAI {event} event: {message}")]
    StreamFailed {
        /// Provider event type.
        event: String,
        /// Provider-reported error text.
        message: String,
    },
    /// The endpoint response did not satisfy the selected protocol.
    #[error("invalid OpenAI response: {0}")]
    InvalidResponse(String),
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
