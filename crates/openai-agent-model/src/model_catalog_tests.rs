use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

use crate::test_support::start_server;
use crate::{ModelConfig, ModelDescriptor, ModelError, OpenAiModel};

#[tokio::test]
async fn lists_models_in_sorted_deduplicated_order_from_a_complete_endpoint(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = ModelCatalogState::new(
        StatusCode::OK,
        r#"{
            "object": "list",
            "data": [
                {"id": "fixture-reasoning", "owned_by": "ahp-test"},
                {"id": "fixture-fast", "owned_by": "ahp-test"},
                {"id": "fixture-fast", "owned_by": "ahp-test"},
                {"id": "fixture-model"}
            ]
        }"#,
    );
    let address = start_server(app(state.clone())).await?;
    let model = OpenAiModel::new(
        ModelConfig::new(format!("http://{address}/v1/responses")).with_api_key("catalog-token"),
    )?;

    assert_eq!(
        model.list_models().await?,
        vec![
            ModelDescriptor {
                id: "fixture-fast".to_owned(),
                owned_by: Some("ahp-test".to_owned()),
            },
            ModelDescriptor {
                id: "fixture-model".to_owned(),
                owned_by: None,
            },
            ModelDescriptor {
                id: "fixture-reasoning".to_owned(),
                owned_by: Some("ahp-test".to_owned()),
            },
        ]
    );
    assert_eq!(
        state.authorization(),
        Some("Bearer catalog-token".to_owned())
    );
    Ok(())
}

#[tokio::test]
async fn distinguishes_unsupported_model_catalog_from_auth_and_malformed_responses(
) -> Result<(), Box<dyn std::error::Error>> {
    for status in [
        StatusCode::NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED,
        StatusCode::NOT_IMPLEMENTED,
    ] {
        let address = start_server(app(ModelCatalogState::new(status, "unsupported"))).await?;
        let error = client(address)?.list_models().await.expect_err("must fail");
        assert!(matches!(
            error,
            ModelError::ModelsUnsupported {
                status: actual, ..
            } if actual == status
        ));
    }

    let address = start_server(app(ModelCatalogState::new(
        StatusCode::UNAUTHORIZED,
        "bad credentials",
    )))
    .await?;
    let error = client(address)?.list_models().await.expect_err("must fail");
    assert!(matches!(
        error,
        ModelError::Upstream {
            status: StatusCode::UNAUTHORIZED,
            ..
        }
    ));

    let address = start_server(app(ModelCatalogState::new(
        StatusCode::OK,
        r#"{"data":"not an array"}"#,
    )))
    .await?;
    let error = client(address)?.list_models().await.expect_err("must fail");
    assert!(matches!(error, ModelError::InvalidResponse(_)));
    Ok(())
}

fn client(address: std::net::SocketAddr) -> Result<OpenAiModel, ModelError> {
    OpenAiModel::new(ModelConfig::new(format!("http://{address}")))
}

fn app(state: ModelCatalogState) -> Router {
    Router::new()
        .route("/v1/models", get(models))
        .with_state(state)
}

#[derive(Clone)]
struct ModelCatalogState {
    status: StatusCode,
    body: Arc<String>,
    authorization: Arc<Mutex<Option<String>>>,
}

impl ModelCatalogState {
    fn new(status: StatusCode, body: impl Into<String>) -> Self {
        Self {
            status,
            body: Arc::new(body.into()),
            authorization: Arc::new(Mutex::new(None)),
        }
    }

    fn authorization(&self) -> Option<String> {
        match self.authorization.lock() {
            Ok(authorization) => authorization.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

async fn models(State(state): State<ModelCatalogState>, headers: HeaderMap) -> Response {
    let authorization = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    match state.authorization.lock() {
        Ok(mut stored) => *stored = authorization,
        Err(poisoned) => *poisoned.into_inner() = authorization,
    }
    (state.status, state.body.as_str().to_owned()).into_response()
}
