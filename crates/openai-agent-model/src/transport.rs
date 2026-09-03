//! API selection, HTTP dispatch, and bounded upstream-error collection.

use std::time::Duration;

use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use serde::Deserialize;
use tokio::time::{timeout, Instant};

use crate::dto::request_json;
use crate::sse::response_stream;
use crate::{ModelApi, ModelDescriptor, ModelError, ModelEventStream, ModelRequest, OpenAiModel};

const MAX_ERROR_BODY_BYTES: usize = 4_096;
const MAX_MODELS_BODY_BYTES: usize = 1_048_576;

#[derive(Deserialize)]
struct ModelCatalogResponse {
    data: Vec<ModelDescriptor>,
}

pub(crate) async fn list_models(client: &OpenAiModel) -> Result<Vec<ModelDescriptor>, ModelError> {
    let response = send_model_catalog_request(client).await?;
    let status = response.status();
    if supports_fallback(status) {
        return Err(ModelError::ModelsUnsupported {
            status,
            body: bounded_body(response, client.config.timeouts.idle).await?,
        });
    }
    if !status.is_success() {
        return Err(http_error(response, client.config.timeouts.idle).await?);
    }
    let bytes = bounded_model_catalog_body(response, client.config.timeouts.idle).await?;
    let mut catalog: ModelCatalogResponse = serde_json::from_slice(&bytes).map_err(|error| {
        ModelError::InvalidResponse(format!(
            "model catalog response was not valid JSON: {error}"
        ))
    })?;
    if catalog.data.iter().any(|model| model.id.trim().is_empty()) {
        return Err(ModelError::InvalidResponse(
            "model catalog response contained an empty model identifier".to_owned(),
        ));
    }
    catalog.data.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.owned_by.cmp(&right.owned_by))
    });
    catalog.data.dedup_by(|left, right| left.id == right.id);
    Ok(catalog.data)
}

pub(crate) async fn open(
    client: &OpenAiModel,
    request: &ModelRequest,
) -> Result<ModelEventStream, ModelError> {
    let api = client.endpoints.forced_api().unwrap_or(client.config.api);
    match api {
        ModelApi::Responses | ModelApi::ChatCompletions => open_api(client, api, request).await,
        ModelApi::Auto => open_auto(client, request).await,
    }
}

async fn open_auto(
    client: &OpenAiModel,
    request: &ModelRequest,
) -> Result<ModelEventStream, ModelError> {
    if let Some(api) = client.cached_api() {
        return open_api(client, api, request).await;
    }
    let response = send(client, ModelApi::Responses, request).await?;
    if response.status().is_success() {
        validate_sse(&response)?;
        client.cache_api(ModelApi::Responses);
        return Ok(response_stream(
            ModelApi::Responses,
            response,
            client.config.timeouts.idle,
        ));
    }
    if supports_fallback(response.status()) {
        drop(response);
        client.cache_api(ModelApi::ChatCompletions);
        return open_api(client, ModelApi::ChatCompletions, request).await;
    }
    Err(http_error(response, client.config.timeouts.idle).await?)
}

async fn open_api(
    client: &OpenAiModel,
    api: ModelApi,
    request: &ModelRequest,
) -> Result<ModelEventStream, ModelError> {
    let response = send(client, api, request).await?;
    if !response.status().is_success() {
        return Err(http_error(response, client.config.timeouts.idle).await?);
    }
    validate_sse(&response)?;
    Ok(response_stream(api, response, client.config.timeouts.idle))
}

async fn send(
    client: &OpenAiModel,
    api: ModelApi,
    request: &ModelRequest,
) -> Result<reqwest::Response, ModelError> {
    let mut builder = client
        .client
        .post(client.endpoints.url(api))
        .header(ACCEPT, "text/event-stream")
        .json(&request_json(api, client.config.model.as_deref(), request)?);
    if let Some(api_key) = &client.config.api_key {
        builder = builder.bearer_auth(api_key);
    }
    timeout(client.config.timeouts.connect, builder.send())
        .await
        .map_err(|_| ModelError::ResponseTimeout {
            timeout: client.config.timeouts.connect,
        })?
        .map_err(ModelError::Request)
}

async fn send_model_catalog_request(client: &OpenAiModel) -> Result<reqwest::Response, ModelError> {
    let mut builder = client
        .client
        .get(client.endpoints.models())
        .header(ACCEPT, "application/json");
    if let Some(api_key) = &client.config.api_key {
        builder = builder.bearer_auth(api_key);
    }
    timeout(client.config.timeouts.connect, builder.send())
        .await
        .map_err(|_| ModelError::ResponseTimeout {
            timeout: client.config.timeouts.connect,
        })?
        .map_err(ModelError::Request)
}

fn supports_fallback(status: reqwest::StatusCode) -> bool {
    matches!(
        status,
        reqwest::StatusCode::NOT_FOUND
            | reqwest::StatusCode::METHOD_NOT_ALLOWED
            | reqwest::StatusCode::NOT_IMPLEMENTED
    )
}

fn validate_sse(response: &reqwest::Response) -> Result<(), ModelError> {
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("text/event-stream"))
    {
        Ok(())
    } else {
        Err(ModelError::InvalidResponse(
            "successful OpenAI stream did not use text/event-stream".to_owned(),
        ))
    }
}

async fn http_error(response: reqwest::Response, idle: Duration) -> Result<ModelError, ModelError> {
    let status = response.status();
    Ok(ModelError::Upstream {
        status,
        body: bounded_body(response, idle).await?,
    })
}

async fn bounded_body(response: reqwest::Response, idle: Duration) -> Result<String, ModelError> {
    let body = response.bytes_stream();
    futures_util::pin_mut!(body);
    let mut bytes = Vec::new();
    let mut deadline = Instant::now() + idle;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next = timeout(remaining, body.next())
            .await
            .map_err(|_| ModelError::IdleTimeout { timeout: idle })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }
        deadline = Instant::now() + idle;
        let available = MAX_ERROR_BODY_BYTES.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..chunk.len().min(available)]);
        if bytes.len() == MAX_ERROR_BODY_BYTES {
            break;
        }
    }
    let text = String::from_utf8_lossy(&bytes);
    Ok(if text.len() < MAX_ERROR_BODY_BYTES {
        text.into_owned()
    } else {
        format!("{text}…")
    })
}

async fn bounded_model_catalog_body(
    response: reqwest::Response,
    idle: Duration,
) -> Result<Vec<u8>, ModelError> {
    let body = response.bytes_stream();
    futures_util::pin_mut!(body);
    let mut bytes = Vec::new();
    let mut deadline = Instant::now() + idle;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let next = timeout(remaining, body.next())
            .await
            .map_err(|_| ModelError::IdleTimeout { timeout: idle })?;
        let Some(chunk) = next else {
            break;
        };
        let chunk = chunk?;
        if chunk.is_empty() {
            continue;
        }
        deadline = Instant::now() + idle;
        let available = MAX_MODELS_BODY_BYTES.saturating_sub(bytes.len());
        if chunk.len() > available {
            return Err(ModelError::InvalidResponse(format!(
                "model catalog response exceeded {MAX_MODELS_BODY_BYTES} bytes"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod selection_tests;

#[cfg(test)]
#[path = "model_catalog_tests.rs"]
mod model_catalog_tests;

#[cfg(test)]
#[path = "timeout_tests.rs"]
mod timeout_tests;
