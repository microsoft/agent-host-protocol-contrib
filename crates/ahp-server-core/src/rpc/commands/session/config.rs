//! Root commands for resolving and completing the fixed model configuration.

use ahp_types::commands::{
    ResolveSessionConfigParams, SessionConfigCompletionsParams, SessionConfigCompletionsResult,
    SessionConfigValueItem,
};
use ahp_types::common::ROOT_RESOURCE_URI;
use ahp_types::errors::ahp_error_codes;
use serde_json::{json, Value};

use crate::host::CoreHost;
use crate::rpc::RpcFault;

pub(in crate::rpc::commands) fn resolve_session_config(
    host: &CoreHost,
    request: ResolveSessionConfigParams,
) -> Result<Value, RpcFault> {
    validate_context(
        host,
        &request.channel,
        request.provider.as_deref(),
        request.working_directory,
    )?;
    let selection = host
        .inner
        .model_catalog
        .resolve(request.config.as_ref())
        .map_err(|error| RpcFault::invalid_params(error.to_string()))?;
    Ok(json!({
        "schema": host.inner.model_catalog.session_config_schema_for(&selection),
        "values": crate::ModelCatalog::session_config_values(&selection),
    }))
}

pub(in crate::rpc::commands) fn session_config_completions(
    host: &CoreHost,
    request: SessionConfigCompletionsParams,
) -> Result<Value, RpcFault> {
    validate_context(
        host,
        &request.channel,
        request.provider.as_deref(),
        request.working_directory,
    )?;
    if request.property != "model" && request.property != "modelId" {
        return Err(RpcFault::invalid_params(
            "sessionConfigCompletions supports only model or modelId",
        ));
    }
    let query = request.query.unwrap_or_default().to_ascii_lowercase();
    let items = host
        .inner
        .model_catalog
        .models
        .iter()
        .filter(|model| {
            query.is_empty()
                || model.id.to_ascii_lowercase().contains(&query)
                || model.name.to_ascii_lowercase().contains(&query)
        })
        .map(|model| SessionConfigValueItem {
            value: model.id.clone(),
            label: model.name.clone(),
            description: Some(format!("OpenAI-compatible model {}", model.id)),
        })
        .collect();
    serde_json::to_value(SessionConfigCompletionsResult { items }).map_err(RpcFault::internal)
}

pub(in crate::rpc::commands) fn create_session_config(
    config: Option<&ahp_types::common::JsonObject>,
) -> Result<Option<&ahp_types::common::JsonObject>, RpcFault> {
    let Some(config) = config else {
        return Ok(None);
    };
    let Some(model) = config.get("model").and_then(Value::as_object) else {
        return Err(RpcFault::invalid_params(
            "createSession config must contain a resolved model.id selection",
        ));
    };
    if model.get("id").and_then(Value::as_str).is_none() {
        return Err(RpcFault::invalid_params(
            "createSession config must contain a resolved model.id selection",
        ));
    }
    if model
        .get("config")
        .and_then(Value::as_object)
        .is_some_and(serde_json::Map::is_empty)
    {
        return Err(RpcFault::invalid_params(
            "omit an empty model config instead of sending config: {}",
        ));
    }
    Ok(Some(config))
}

fn validate_context(
    host: &CoreHost,
    channel: &str,
    provider: Option<&str>,
    working_directory: Option<String>,
) -> Result<(), RpcFault> {
    if channel != ROOT_RESOURCE_URI {
        return Err(RpcFault::invalid_params(
            "session configuration commands must target ahp-root://",
        ));
    }
    if let Some(provider) = provider.filter(|provider| *provider != "openai") {
        return Err(RpcFault {
            code: ahp_error_codes::PROVIDER_NOT_FOUND,
            message: format!("no agent registered for provider {provider:?}"),
            data: None,
        });
    }
    if let Some(working_directory) = working_directory {
        host.inner
            .profile
            .select_workspace(Some(vec![working_directory]))
            .map_err(|error| RpcFault::invalid_params(error.to_string()))?;
    }
    Ok(())
}
