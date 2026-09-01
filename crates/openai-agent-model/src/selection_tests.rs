use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{header::CONTENT_TYPE, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Router;

use crate::test_support::start_server;
use crate::{AgentModel, ModelApi, ModelConfig, ModelError, OpenAiModel};

#[tokio::test]
async fn auto_falls_back_only_for_documented_unsupported_statuses(
) -> Result<(), Box<dyn std::error::Error>> {
    for status in [
        StatusCode::NOT_FOUND,
        StatusCode::METHOD_NOT_ALLOWED,
        StatusCode::NOT_IMPLEMENTED,
    ] {
        let state = TestState::new(status);
        let address = start_server(app(state.clone())).await?;
        let client = client(address, ModelApi::Auto)?;
        assert_eq!(client.complete_text("x").await?, "chat");
        assert_eq!(state.responses.load(Ordering::Relaxed), 1);
        assert_eq!(state.chat.load(Ordering::Relaxed), 1);
    }
    for status in [
        StatusCode::BAD_REQUEST,
        StatusCode::UNAUTHORIZED,
        StatusCode::FORBIDDEN,
        StatusCode::REQUEST_TIMEOUT,
        StatusCode::CONFLICT,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::INTERNAL_SERVER_ERROR,
    ] {
        let state = TestState::new(status);
        let address = start_server(app(state.clone())).await?;
        let error = client(address, ModelApi::Auto)?
            .complete_text("x")
            .await
            .expect_err("non-unsupported status must not fall back");
        assert!(matches!(error, ModelError::Upstream { status: actual, .. } if actual == status));
        assert_eq!(state.responses.load(Ordering::Relaxed), 1);
        assert_eq!(state.chat.load(Ordering::Relaxed), 0);
    }
    Ok(())
}

#[tokio::test]
async fn auto_caches_responses_or_fallback_across_clones() -> Result<(), Box<dyn std::error::Error>>
{
    let state = TestState::new(StatusCode::OK);
    let responses_client = client(start_server(app(state.clone())).await?, ModelApi::Auto)?;
    assert_eq!(responses_client.complete_text("first").await?, "responses");
    assert_eq!(
        responses_client.clone().complete_text("second").await?,
        "responses"
    );
    assert_eq!(state.responses.load(Ordering::Relaxed), 2);
    assert_eq!(state.chat.load(Ordering::Relaxed), 0);

    let state = TestState::new(StatusCode::NOT_FOUND);
    let client = client(start_server(app(state.clone())).await?, ModelApi::Auto)?;
    assert_eq!(client.complete_text("first").await?, "chat");
    assert_eq!(client.clone().complete_text("second").await?, "chat");
    assert_eq!(state.responses.load(Ordering::Relaxed), 1);
    assert_eq!(state.chat.load(Ordering::Relaxed), 2);
    Ok(())
}

#[tokio::test]
async fn explicit_and_complete_endpoint_selection_never_probes(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = TestState::new(StatusCode::NOT_FOUND);
    let address = start_server(app(state.clone())).await?;
    let error = client(address, ModelApi::Responses)?
        .complete_text("x")
        .await
        .expect_err("explicit Responses must retain 404");
    assert!(matches!(
        error,
        ModelError::Upstream {
            status: StatusCode::NOT_FOUND,
            ..
        }
    ));
    assert_eq!(state.responses.load(Ordering::Relaxed), 1);
    assert_eq!(state.chat.load(Ordering::Relaxed), 0);

    let state = TestState::new(StatusCode::INTERNAL_SERVER_ERROR);
    let address = start_server(app(state.clone())).await?;
    let full_chat_endpoint = format!("http://{address}/v1/chat/completions");
    let client = OpenAiModel::new(
        ModelConfig::new(full_chat_endpoint, "test").with_api(ModelApi::Responses),
    )?;
    let model: &dyn AgentModel = &client;
    assert_eq!(model.model_name(), "test");
    assert_eq!(client.complete_text("x").await?, "chat");
    assert_eq!(state.responses.load(Ordering::Relaxed), 0);
    assert_eq!(state.chat.load(Ordering::Relaxed), 1);
    Ok(())
}

#[tokio::test]
async fn auto_never_falls_back_after_a_successful_malformed_responses_stream(
) -> Result<(), Box<dyn std::error::Error>> {
    let state = StreamFailureState {
        responses: Arc::new(AtomicUsize::new(0)),
        chat: Arc::new(AtomicUsize::new(0)),
    };
    let address = start_server(
        Router::new()
            .route("/v1/responses", post(malformed_responses))
            .route("/v1/chat/completions", post(unused_chat))
            .with_state(state.clone()),
    )
    .await?;
    let error = client(address, ModelApi::Auto)?
        .complete_text("x")
        .await
        .expect_err("a malformed Responses stream must fail without fallback");
    assert!(matches!(error, ModelError::InvalidResponse(_)));
    assert_eq!(state.responses.load(Ordering::Relaxed), 1);
    assert_eq!(state.chat.load(Ordering::Relaxed), 0);
    Ok(())
}

fn client(address: std::net::SocketAddr, api: ModelApi) -> Result<OpenAiModel, ModelError> {
    OpenAiModel::new(ModelConfig::new(format!("http://{address}"), "test").with_api(api))
}

fn app(state: TestState) -> Router {
    Router::new()
        .route("/v1/responses", post(responses))
        .route("/v1/chat/completions", post(chat))
        .with_state(state)
}

#[derive(Clone)]
struct TestState {
    responses_status: StatusCode,
    responses: Arc<AtomicUsize>,
    chat: Arc<AtomicUsize>,
}

#[derive(Clone)]
struct StreamFailureState {
    responses: Arc<AtomicUsize>,
    chat: Arc<AtomicUsize>,
}

impl TestState {
    fn new(responses_status: StatusCode) -> Self {
        Self {
            responses_status,
            responses: Arc::new(AtomicUsize::new(0)),
            chat: Arc::new(AtomicUsize::new(0)),
        }
    }
}

async fn responses(State(state): State<TestState>) -> Response {
    state.responses.fetch_add(1, Ordering::Relaxed);
    if state.responses_status != StatusCode::OK {
        return (state.responses_status, "responses unavailable").into_response();
    }
    responses_sse("responses")
}

async fn chat(State(state): State<TestState>) -> Response {
    state.chat.fetch_add(1, Ordering::Relaxed);
    chat_sse("chat")
}

async fn malformed_responses(State(state): State<StreamFailureState>) -> Response {
    state.responses.fetch_add(1, Ordering::Relaxed);
    (
        [(CONTENT_TYPE, "text/event-stream")],
        "event: response.output_text.delta\ndata: not-json\n\n",
    )
        .into_response()
}

async fn unused_chat(State(state): State<StreamFailureState>) -> Response {
    state.chat.fetch_add(1, Ordering::Relaxed);
    chat_sse("unexpected")
}

fn responses_sse(text: &str) -> Response {
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "event: response.output_text.delta\ndata: {{\"type\":\"response.output_text.delta\",\"delta\":{text:?}}}\n\n\
             event: response.completed\ndata: {{\"type\":\"response.completed\"}}\n\n"
        ),
    )
        .into_response()
}

fn chat_sse(text: &str) -> Response {
    (
        [(CONTENT_TYPE, "text/event-stream")],
        format!(
            "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":{text:?}}},\"finish_reason\":null}}]}}\n\n\
             data: {{\"choices\":[{{\"index\":0,\"delta\":{{}},\"finish_reason\":\"stop\"}}]}}\n\n\
             data: [DONE]\n\n"
        ),
    )
        .into_response()
}
