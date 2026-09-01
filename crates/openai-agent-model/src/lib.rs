//! A streaming adapter for OpenAI Responses and Chat Completions endpoints.

#![forbid(unsafe_code)]

mod chat;
mod chat_helpers;
mod client;
mod collect;
mod config;
mod dto;
mod endpoint;
mod responses;
mod responses_helpers;
mod sse;
mod transport;
mod types;

pub use client::{AgentModel, ModelClient, ModelRequestFuture, OpenAiModel};
pub use collect::collect_stream;
pub use config::{ModelApi, ModelApiParseError, ModelConfig, StreamTimeouts};
pub use types::{
    ModelError, ModelEventStream, ModelMessage, ModelReply, ModelRequest, ModelStreamEvent,
    ModelToolCall, ModelToolDefinition,
};

#[cfg(test)]
mod test_support;
