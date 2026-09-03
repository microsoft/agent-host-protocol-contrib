//! Public model port and OpenAI client implementation.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::collect_stream;
use crate::endpoint::EndpointUrls;
use crate::transport;
use crate::{
    ModelApi, ModelConfig, ModelError, ModelEventStream, ModelReply, ModelRequest, StreamTimeouts,
};

/// The future returned when an [`AgentModel`] starts a streaming request.
pub type ModelRequestFuture<'a> =
    Pin<Box<dyn Future<Output = Result<ModelEventStream, ModelError>> + Send + 'a>>;

/// A streaming model port suitable for application dependency injection.
pub trait AgentModel: Send + Sync {
    /// Returns the model used when a request does not select one explicitly.
    fn default_model(&self) -> Option<&str>;

    /// Stable upstream scope for persisted Responses API continuation IDs.
    fn response_chain_scope(&self) -> Option<&str> {
        None
    }

    /// Starts an incrementally decoded completion request.
    fn stream<'a>(&'a self, request: &'a ModelRequest) -> ModelRequestFuture<'a>;
}

/// An OpenAI-compatible Responses and Chat Completions streaming client.
#[derive(Clone)]
pub struct OpenAiModel {
    pub(crate) client: reqwest::Client,
    pub(crate) config: ModelConfig,
    pub(crate) endpoints: EndpointUrls,
    auto_selection: Arc<Mutex<Option<ModelApi>>>,
    response_chain_scope: Option<String>,
}

/// Compatibility name for [`OpenAiModel`].
pub type ModelClient = OpenAiModel;

impl OpenAiModel {
    /// Builds a client using the API selection embedded in `config`.
    pub fn new(config: ModelConfig) -> Result<Self, ModelError> {
        validate_config(&config)?;
        let endpoints = EndpointUrls::parse(&config.base_url)?;
        let client = reqwest::Client::builder()
            .connect_timeout(config.timeouts.connect)
            .build()?;
        let responses_available = endpoints.forced_api() != Some(ModelApi::ChatCompletions)
            && config.api != ModelApi::ChatCompletions;
        let response_chain_scope = (config.response_chaining && responses_available)
            .then(|| endpoints.url(ModelApi::Responses).to_owned());
        Ok(Self {
            client,
            config,
            endpoints,
            auto_selection: Arc::new(Mutex::new(None)),
            response_chain_scope,
        })
    }

    /// Compatibility constructor for callers that selected the API separately.
    pub fn new_with_api(mut config: ModelConfig, api: ModelApi) -> Result<Self, ModelError> {
        config.api = api;
        Self::new(config)
    }

    /// Returns the model used when a request does not select one explicitly.
    #[must_use]
    pub fn default_model(&self) -> Option<&str> {
        self.config.model.as_deref()
    }

    /// Returns the configured protocol selection before full-endpoint implication.
    pub const fn configured_api(&self) -> ModelApi {
        self.config.api
    }

    /// Lists models advertised by the configured OpenAI-compatible endpoint.
    ///
    /// A [`ModelError::ModelsUnsupported`] result means the endpoint returned
    /// 404, 405, or 501 and callers may explicitly use their configured
    /// default model instead.
    pub async fn list_models(&self) -> Result<Vec<crate::ModelDescriptor>, ModelError> {
        transport::list_models(self).await
    }

    /// Starts a streaming request. Dropping the returned stream cancels its
    /// HTTP response body.
    pub async fn stream(&self, request: &ModelRequest) -> Result<ModelEventStream, ModelError> {
        transport::open(self, request).await
    }

    /// Collects one completion for consumers that do not need deltas.
    pub async fn complete(&self, request: &ModelRequest) -> Result<ModelReply, ModelError> {
        collect_stream(self.stream(request).await?).await
    }

    /// Sends one user prompt and returns its text-only completion.
    pub async fn complete_text(&self, prompt: &str) -> Result<String, ModelError> {
        let reply = self.complete(&ModelRequest::text(prompt)).await?;
        if !reply.tool_calls.is_empty() {
            return Err(ModelError::InvalidResponse(
                "text-only completion returned tool calls".to_owned(),
            ));
        }
        reply.content.ok_or_else(|| {
            ModelError::InvalidResponse("text-only completion returned null content".to_owned())
        })
    }

    pub(crate) fn cached_api(&self) -> Option<ModelApi> {
        match self.auto_selection.lock() {
            Ok(selection) => *selection,
            Err(poisoned) => *poisoned.into_inner(),
        }
    }

    pub(crate) fn cache_api(&self, api: ModelApi) {
        match self.auto_selection.lock() {
            Ok(mut selection) => *selection = Some(api),
            Err(poisoned) => *poisoned.into_inner() = Some(api),
        }
    }
}

impl AgentModel for OpenAiModel {
    fn default_model(&self) -> Option<&str> {
        Self::default_model(self)
    }

    fn response_chain_scope(&self) -> Option<&str> {
        self.response_chain_scope.as_deref()
    }

    fn stream<'a>(&'a self, request: &'a ModelRequest) -> ModelRequestFuture<'a> {
        Box::pin(Self::stream(self, request))
    }
}

fn validate_config(config: &ModelConfig) -> Result<(), ModelError> {
    if config.base_url.trim().is_empty() {
        return Err(ModelError::InvalidBaseUrl {
            base_url: config.base_url.clone(),
            reason: "the URL must not be empty".to_owned(),
        });
    }
    if config
        .model
        .as_deref()
        .is_some_and(|model| model.trim().is_empty())
    {
        return Err(ModelError::InvalidResponse(
            "a configured default model must not be empty".to_owned(),
        ));
    }
    validate_timeouts(config.timeouts)
}

fn validate_timeouts(timeouts: StreamTimeouts) -> Result<(), ModelError> {
    if timeouts.connect.is_zero() {
        return Err(ModelError::InvalidTimeout { name: "connect" });
    }
    if timeouts.idle.is_zero() {
        return Err(ModelError::InvalidTimeout { name: "idle" });
    }
    Ok(())
}
