//! Model-catalog construction from OpenAI model-list descriptors.

use ahp_types::state::{ModelSelection, SessionModelInfo};
use openai_agent_model::ModelDescriptor;

use super::{ModelCatalog, ModelCatalogError};

impl ModelCatalog {
    /// Maps discovered OpenAI model descriptors to standard catalogue entries.
    #[must_use]
    pub fn from_model_descriptors<I>(models: I, default_id: impl Into<String>) -> Self
    where
        I: IntoIterator<Item = ModelDescriptor>,
    {
        Self::new(
            models.into_iter().map(|model| (model.id.clone(), model.id)),
            default_id,
        )
    }

    /// Maps descriptors using a caller-supplied model capability policy.
    ///
    /// The OpenAI model-list response does not expose reasoning capabilities, so
    /// composition roots supply them instead of this reusable crate guessing
    /// from model IDs.
    pub fn from_model_descriptors_with_thinking_levels<I, F, Levels, Level>(
        models: I,
        default_id: impl Into<String>,
        mut thinking_levels: F,
    ) -> Result<Self, ModelCatalogError>
    where
        I: IntoIterator<Item = ModelDescriptor>,
        F: FnMut(&ModelDescriptor) -> Levels,
        Levels: IntoIterator<Item = Level>,
        Level: Into<String>,
    {
        let models = models
            .into_iter()
            .map(|model| {
                Self::model_info_from_descriptor_with_thinking_levels(
                    &model,
                    thinking_levels(&model),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            models,
            default_selection: ModelSelection {
                id: default_id.into(),
                config: None,
            },
        })
    }

    /// Maps one discovered OpenAI model descriptor to a standard catalogue entry.
    #[must_use]
    pub fn model_info_from_descriptor(model: &ModelDescriptor) -> SessionModelInfo {
        Self::model_info(&model.id, &model.id)
    }

    /// Maps one discovered descriptor with its supported thinking-level subset.
    pub fn model_info_from_descriptor_with_thinking_levels<I, Level>(
        model: &ModelDescriptor,
        thinking_levels: I,
    ) -> Result<SessionModelInfo, ModelCatalogError>
    where
        I: IntoIterator<Item = Level>,
        Level: Into<String>,
    {
        Self::model_info_with_thinking_levels(model.id.clone(), model.id.clone(), thinking_levels)
    }
}
