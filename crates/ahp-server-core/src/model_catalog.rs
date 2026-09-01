//! Model catalogue validation and AHP session-configuration projection.

use std::collections::{HashMap, HashSet};

use ahp_types::common::JsonObject;
use ahp_types::state::{
    ModelSelection, SessionConfigPropertySchema, SessionConfigSchema, SessionConfigState,
    SessionModelInfo,
};
use serde_json::Value;
use thiserror::Error;

mod descriptors;
mod schema;

use schema::{
    model_info, model_selection_properties, selection_value, standard_thinking_levels,
    thinking_levels_for, valid_thinking_level,
};

/// The models available to an AHP host and its default session selection.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelCatalog {
    /// Models advertised in the root snapshot.
    pub models: Vec<SessionModelInfo>,
    /// Model and optional reasoning preference used when a caller omits one.
    pub default_selection: ModelSelection,
}

impl ModelCatalog {
    /// Creates one OpenAI-compatible model with the standard thinking-level schema.
    #[must_use]
    pub fn single(id: impl Into<String>) -> Self {
        let id = id.into();
        Self::new([(id.clone(), id.clone())], id)
    }

    /// Creates a catalogue whose models share the standard thinking-level schema.
    ///
    /// Construction is infallible so composition roots can build a catalogue
    /// directly from discovered models. [`Self::validate`] is called by
    /// [`crate::CoreHost::with_model_catalog`] before it accepts peers.
    #[must_use]
    pub fn new<I, Id, Name>(models: I, default_id: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = (Id, Name)>,
        Id: Into<String>,
        Name: Into<String>,
    {
        Self {
            models: models
                .into_iter()
                .map(|(id, name)| Self::model_info(id, name))
                .collect(),
            default_selection: ModelSelection {
                id: default_id.into(),
                config: None,
            },
        }
    }

    /// Builds one OpenAI model descriptor with the standard thinking-level schema.
    #[must_use]
    pub fn model_info(id: impl Into<String>, name: impl Into<String>) -> SessionModelInfo {
        model_info(id.into(), name.into(), &standard_thinking_levels())
    }

    /// Builds an OpenAI model descriptor with only its supported thinking levels.
    ///
    /// An empty level list advertises no model-specific configuration. Every
    /// non-empty level must be one of the protocol's standard values.
    pub fn model_info_with_thinking_levels<I, Level>(
        id: impl Into<String>,
        name: impl Into<String>,
        thinking_levels: I,
    ) -> Result<SessionModelInfo, ModelCatalogError>
    where
        I: IntoIterator<Item = Level>,
        Level: Into<String>,
    {
        let thinking_levels = thinking_levels
            .into_iter()
            .map(Into::into)
            .collect::<Vec<_>>();
        validate_thinking_levels(&thinking_levels)?;
        Ok(model_info(id.into(), name.into(), &thinking_levels))
    }

    /// Adds a default model selection with no model-specific config value.
    #[must_use]
    pub fn with_default_selection(self, default_selection: ModelSelection) -> Self {
        Self {
            models: self.models,
            default_selection,
        }
    }

    /// Verifies that this catalogue can be exposed and selected safely.
    pub fn validate(&self) -> Result<(), ModelCatalogError> {
        if self.models.is_empty() {
            return Err(ModelCatalogError::Empty);
        }
        let mut ids = HashSet::new();
        for model in &self.models {
            if model.id.trim().is_empty() {
                return Err(ModelCatalogError::EmptyId);
            }
            if !ids.insert(&model.id) {
                return Err(ModelCatalogError::DuplicateId(model.id.clone()));
            }
            if model.provider != "openai" {
                return Err(ModelCatalogError::Provider(model.provider.clone()));
            }
            if let Some(levels) = thinking_levels_for(model) {
                validate_thinking_levels(&levels)?;
            }
        }
        self.validate_selection(&self.default_selection)
    }

    /// Resolves a partial protocol configuration to its canonical model selection.
    pub fn resolve(
        &self,
        values: Option<&JsonObject>,
    ) -> Result<ModelSelection, ModelCatalogError> {
        let Some(values) = values else {
            return Ok(self.default_selection.clone());
        };
        if values.keys().any(|key| key != "model") {
            return Err(ModelCatalogError::UnsupportedProperty);
        }
        let Some(model) = values.get("model") else {
            return Ok(self.default_selection.clone());
        };
        let model = model.as_object().ok_or(ModelCatalogError::InvalidModel)?;
        if model.keys().any(|key| key != "id" && key != "config") {
            return Err(ModelCatalogError::InvalidModel);
        }
        let id = match model.get("id") {
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(ModelCatalogError::InvalidModel),
            None => self.default_selection.id.clone(),
        };
        let config = match model.get("config") {
            None if id == self.default_selection.id => self.default_selection.config.clone(),
            None => None,
            Some(Value::Object(config)) if config.is_empty() => None,
            Some(Value::Object(config)) => Some(
                config
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect(),
            ),
            Some(_) => return Err(ModelCatalogError::InvalidConfig),
        };
        let selection = ModelSelection { id, config };
        self.validate_selection(&selection)?;
        Ok(selection)
    }

    /// Validates a fully resolved selection supplied by a message or store record.
    pub fn validate_selection(&self, selection: &ModelSelection) -> Result<(), ModelCatalogError> {
        let model = self
            .models
            .iter()
            .find(|model| model.id == selection.id)
            .ok_or_else(|| ModelCatalogError::UnknownModel(selection.id.clone()))?;
        let Some(config) = &selection.config else {
            return Ok(());
        };
        if config.is_empty() || config.keys().any(|key| key != "thinkingLevel") {
            return Err(ModelCatalogError::InvalidConfig);
        }
        let Some(Value::String(level)) = config.get("thinkingLevel") else {
            return Err(ModelCatalogError::InvalidThinkingLevel);
        };
        if thinking_levels_for(model)
            .is_some_and(|levels| levels.iter().any(|allowed| allowed == level))
        {
            Ok(())
        } else {
            Err(ModelCatalogError::InvalidThinkingLevel)
        }
    }

    /// Produces the resolved session schema advertised by config commands and snapshots.
    #[must_use]
    pub fn session_config_schema(&self) -> SessionConfigSchema {
        self.session_config_schema_for(&self.default_selection)
    }

    /// Produces a schema whose nested thinking-level values match `selection`.
    #[must_use]
    pub fn session_config_schema_for(&self, selection: &ModelSelection) -> SessionConfigSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "model".into(),
            SessionConfigPropertySchema {
                r#type: "object".into(),
                title: "Model".into(),
                description: Some("The model and its reasoning preference.".into()),
                default: Some(selection_value(&self.default_selection)),
                r#enum: None,
                enum_labels: None,
                enum_descriptions: None,
                read_only: None,
                items: None,
                properties: Some(model_selection_properties(&self.models, &selection.id)),
                required: Some(vec!["id".into()]),
                additional_properties: None,
                enum_dynamic: Some(true),
                session_mutable: Some(true),
            },
        );
        SessionConfigSchema {
            r#type: "object".into(),
            properties,
            required: Some(vec!["model".into()]),
        }
    }

    /// Projects one selection as canonical AHP config values.
    #[must_use]
    pub fn session_config_values(selection: &ModelSelection) -> JsonObject {
        JsonObject::from_iter([("model".into(), selection_value(selection))])
    }

    /// Projects the schema and resolved values held by a session.
    #[must_use]
    pub fn session_config_state(&self, selection: &ModelSelection) -> SessionConfigState {
        SessionConfigState {
            schema: self.session_config_schema_for(selection),
            values: Self::session_config_values(selection),
        }
    }
}

fn validate_thinking_levels(levels: &[String]) -> Result<(), ModelCatalogError> {
    let mut unique = HashSet::new();
    if levels
        .iter()
        .any(|level| !valid_thinking_level(level) || !unique.insert(level))
    {
        Err(ModelCatalogError::InvalidThinkingLevel)
    } else {
        Ok(())
    }
}

/// A malformed model catalogue or model-selection value.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ModelCatalogError {
    /// At least one selectable model is required.
    #[error("model catalogue must contain at least one model")]
    Empty,
    /// Model IDs cannot be blank.
    #[error("model IDs must not be empty")]
    EmptyId,
    /// Model IDs identify one catalogue entry each.
    #[error("duplicate model ID {0:?}")]
    DuplicateId(String),
    /// This host exposes only the OpenAI-compatible provider.
    #[error("model provider {0:?} is unsupported")]
    Provider(String),
    /// The selected model is not in the catalogue.
    #[error("unknown model {0:?}")]
    UnknownModel(String),
    /// The `model` configuration property did not have object shape.
    #[error("model must be an object containing an optional id and config")]
    InvalidModel,
    /// Model config has unsupported property keys or invalid shape.
    #[error("model config may contain only thinkingLevel")]
    InvalidConfig,
    /// A thinking-level setting was not a supported string.
    #[error("thinkingLevel must be one of none, minimal, low, medium, high, xhigh, or max")]
    InvalidThinkingLevel,
    /// The host supports only the `model` session configuration property.
    #[error("only the model session configuration property is supported")]
    UnsupportedProperty,
}

#[cfg(test)]
mod tests;
