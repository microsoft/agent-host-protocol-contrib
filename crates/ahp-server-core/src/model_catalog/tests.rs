use super::{ModelCatalog, ModelCatalogError};
use ahp_types::state::ModelSelection;
use serde_json::json;

#[test]
fn projects_model_schema_and_defaults() {
    let catalog = ModelCatalog::single("gpt-test");
    assert!(catalog.validate().is_ok());
    let schema = catalog.session_config_schema();
    assert_eq!(schema.required, Some(vec!["model".into()]));
    assert_eq!(
        schema
            .properties
            .get("model")
            .and_then(|model| model.properties.as_ref())
            .and_then(|model| model.get("config"))
            .and_then(|config| config.properties.as_ref())
            .and_then(|config| config.get("thinkingLevel"))
            .and_then(|thinking| thinking.r#enum.as_ref())
            .map(Vec::len),
        Some(7)
    );
}

#[test]
fn supports_model_specific_thinking_levels() {
    let alpha =
        ModelCatalog::model_info_with_thinking_levels("alpha", "Alpha", ["minimal", "medium"])
            .expect("valid thinking levels");
    let beta = ModelCatalog::model_info_with_thinking_levels("beta", "Beta", ["high"])
        .expect("valid thinking levels");
    let catalog = ModelCatalog {
        models: vec![alpha, beta],
        default_selection: ModelSelection {
            id: "alpha".into(),
            config: None,
        },
    };
    assert!(catalog.validate().is_ok());
    assert!(catalog
        .resolve(Some(
            &serde_json::from_value(json!({
                "model": { "id": "alpha", "config": { "thinkingLevel": "minimal" } }
            }))
            .expect("valid config object")
        ))
        .is_ok());
    assert!(catalog
        .resolve(Some(
            &serde_json::from_value(json!({
                "model": { "id": "alpha", "config": { "thinkingLevel": "high" } }
            }))
            .expect("valid config object")
        ))
        .is_err());
}

#[test]
fn resolves_defaults_and_partial_values() -> Result<(), String> {
    let catalog = ModelCatalog::single("gpt-test");
    assert_eq!(catalog.resolve(None), Ok(catalog.default_selection.clone()));
    let partial = serde_json::from_value(json!({
        "model": { "config": { "thinkingLevel": "medium" } }
    }))
    .map_err(|error| error.to_string())?;
    assert_eq!(
        catalog.resolve(Some(&partial)),
        Ok(ModelSelection {
            id: "gpt-test".into(),
            config: Some(std::collections::HashMap::from([(
                "thinkingLevel".into(),
                json!("medium")
            )])),
        })
    );
    Ok(())
}

#[test]
fn rejects_invalid_selection() -> Result<(), String> {
    let catalog = ModelCatalog::single("gpt-test");
    let invalid = serde_json::from_value(json!({
        "model": { "id": "missing" }
    }))
    .map_err(|error| error.to_string())?;
    assert_eq!(
        catalog.resolve(Some(&invalid)),
        Err(ModelCatalogError::UnknownModel("missing".into()))
    );
    Ok(())
}
