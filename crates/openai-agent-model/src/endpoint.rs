//! Endpoint normalization and full-endpoint API implication.

use crate::{ModelApi, ModelError};

#[derive(Clone)]
pub(crate) struct EndpointUrls {
    responses: String,
    chat_completions: String,
    models: String,
    forced_api: Option<ModelApi>,
}

impl EndpointUrls {
    pub(crate) fn parse(base_url: &str) -> Result<Self, ModelError> {
        let configured_url = base_url.trim();
        let mut url =
            reqwest::Url::parse(configured_url).map_err(|error| ModelError::InvalidBaseUrl {
                base_url: configured_url.to_owned(),
                reason: error.to_string(),
            })?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(ModelError::InvalidBaseUrl {
                base_url: configured_url.to_owned(),
                reason: "the URL must be an absolute HTTP or HTTPS URL".to_owned(),
            });
        }
        if url.fragment().is_some() {
            return Err(ModelError::InvalidBaseUrl {
                base_url: configured_url.to_owned(),
                reason: "the URL must not contain a fragment".to_owned(),
            });
        }
        let path = url.path().trim_end_matches('/').to_owned();
        let forced_api = if path.ends_with("/responses") {
            Some(ModelApi::Responses)
        } else if path.ends_with("/chat/completions") {
            Some(ModelApi::ChatCompletions)
        } else {
            None
        };
        let models = model_catalog_url(&url, &path, forced_api);
        if let Some(api) = forced_api {
            return Ok(match api {
                ModelApi::Responses => Self {
                    responses: url.to_string(),
                    chat_completions: String::new(),
                    models,
                    forced_api,
                },
                ModelApi::ChatCompletions => Self {
                    responses: String::new(),
                    chat_completions: url.to_string(),
                    models,
                    forced_api,
                },
                ModelApi::Auto => {
                    return Err(ModelError::InvalidResponse(
                        "a complete endpoint cannot imply automatic API selection".to_owned(),
                    ));
                }
            });
        }
        let root_path = if path.ends_with("/v1") {
            path
        } else {
            format!("{path}/v1")
        };
        url.set_path(&format!("{root_path}/responses"));
        let responses = url.to_string();
        url.set_path(&format!("{root_path}/chat/completions"));
        Ok(Self {
            responses,
            chat_completions: url.to_string(),
            models,
            forced_api: None,
        })
    }

    pub(crate) const fn forced_api(&self) -> Option<ModelApi> {
        self.forced_api
    }

    pub(crate) fn url(&self, api: ModelApi) -> &str {
        match api {
            ModelApi::Responses | ModelApi::Auto => &self.responses,
            ModelApi::ChatCompletions => &self.chat_completions,
        }
    }

    pub(crate) fn models(&self) -> &str {
        &self.models
    }
}

fn model_catalog_url(url: &reqwest::Url, path: &str, forced_api: Option<ModelApi>) -> String {
    let endpoint_root = match forced_api {
        Some(ModelApi::Responses) => path.strip_suffix("/responses").unwrap_or(path),
        Some(ModelApi::ChatCompletions) => path.strip_suffix("/chat/completions").unwrap_or(path),
        Some(ModelApi::Auto) | None => path,
    };
    let version_path = if endpoint_root.ends_with("/v1") {
        endpoint_root.to_owned()
    } else {
        format!("{endpoint_root}/v1")
    };
    let mut models_url = url.clone();
    models_url.set_path(&format!("{version_path}/models"));
    models_url.to_string()
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
