//! Public requests, events, replies, and errors.

use std::pin::Pin;
use std::time::Duration;

use futures_util::Stream;
use serde_json::Value;
use thiserror::Error;

/// One model completion request, including conversation state and tools.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelRequest {
    /// Ordered conversation items.
    pub messages: Vec<ModelMessage>,
    /// Function tools that the model may call.
    pub tools: Vec<ModelToolDefinition>,
}

impl ModelRequest {
    /// Creates a text-only request with one user message.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            messages: vec![ModelMessage::User {
                content: content.into(),
            }],
            tools: Vec::new(),
        }
    }
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
}

/// One decoded incremental item from a model stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelStreamEvent {
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
