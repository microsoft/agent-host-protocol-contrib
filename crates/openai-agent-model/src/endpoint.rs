//! Endpoint normalization and full-endpoint API implication.

use crate::{ModelApi, ModelError};

#[derive(Clone)]
pub(crate) struct EndpointUrls {
    responses: String,
    chat_completions: String,
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
        let path = url.path().trim_end_matches('/');
        let forced_api = if path.ends_with("/responses") {
            Some(ModelApi::Responses)
        } else if path.ends_with("/chat/completions") {
            Some(ModelApi::ChatCompletions)
        } else {
            None
        };
        if let Some(api) = forced_api {
            return Ok(match api {
                ModelApi::Responses => Self {
                    responses: url.to_string(),
                    chat_completions: String::new(),
                    forced_api,
                },
                ModelApi::ChatCompletions => Self {
                    responses: String::new(),
                    chat_completions: url.to_string(),
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
            path.to_owned()
        } else {
            format!("{path}/v1")
        };
        url.set_path(&format!("{root_path}/responses"));
        let responses = url.to_string();
        url.set_path(&format!("{root_path}/chat/completions"));
        Ok(Self {
            responses,
            chat_completions: url.to_string(),
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
}

#[cfg(test)]
#[path = "endpoint_tests.rs"]
mod tests;
