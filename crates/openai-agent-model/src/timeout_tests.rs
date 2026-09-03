use std::convert::Infallible;
use std::time::Duration;

use async_stream::stream;
use axum::body::{Body, Bytes};
use axum::http::{header::CONTENT_TYPE, HeaderValue};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use futures_util::StreamExt;
use tokio::time::{timeout, Instant};

use crate::test_support::start_server;
use crate::{ModelApi, ModelConfig, ModelError, OpenAiModel, StreamTimeouts};

#[tokio::test]
async fn frequent_chunks_can_outlive_the_idle_timeout_without_a_total_deadline(
) -> Result<(), Box<dyn std::error::Error>> {
    let address =
        start_server(Router::new().route("/v1/responses", post(healthy_but_long_running_stream)))
            .await?;
    let timeouts = StreamTimeouts {
        connect: Duration::from_millis(80),
        idle: Duration::from_millis(80),
    };
    let client = OpenAiModel::new(
        ModelConfig::new(format!("http://{address}"))
            .with_default_model("test")
            .with_api(ModelApi::Responses)
            .with_timeouts(timeouts),
    )?;
    let started = Instant::now();
    assert_eq!(client.complete_text("x").await?, "abc");
    assert!(
        started.elapsed() > timeouts.idle,
        "the successful stream should exceed one idle interval in total"
    );
    Ok(())
}

#[tokio::test]
async fn a_silent_stream_returns_a_typed_idle_timeout() -> Result<(), Box<dyn std::error::Error>> {
    let address = start_server(Router::new().route("/v1/responses", post(silent_stream))).await?;
    let idle = Duration::from_millis(30);
    let client = OpenAiModel::new(
        ModelConfig::new(format!("http://{address}"))
            .with_default_model("test")
            .with_api(ModelApi::Responses)
            .with_timeouts(StreamTimeouts {
                connect: Duration::from_secs(1),
                idle,
            }),
    )?;
    let mut stream = client.stream(&crate::ModelRequest::text("x")).await?;
    let event = timeout(Duration::from_secs(1), stream.next()).await?;
    assert!(matches!(
        event,
        Some(Err(ModelError::IdleTimeout { timeout })) if timeout == idle
    ));
    Ok(())
}

#[tokio::test]
async fn initial_header_wait_is_bounded_by_the_connect_timeout(
) -> Result<(), Box<dyn std::error::Error>> {
    let address = start_server(Router::new().route("/v1/responses", post(delayed_headers))).await?;
    let connect = Duration::from_millis(30);
    let client = OpenAiModel::new(
        ModelConfig::new(format!("http://{address}"))
            .with_default_model("test")
            .with_api(ModelApi::Responses)
            .with_timeouts(StreamTimeouts {
                connect,
                idle: Duration::from_secs(1),
            }),
    )?;
    assert!(matches!(
        client.stream(&crate::ModelRequest::text("x")).await,
        Err(ModelError::ResponseTimeout { timeout }) if timeout == connect
    ));
    Ok(())
}

#[tokio::test]
async fn a_truncated_stream_is_not_treated_as_completed() -> Result<(), Box<dyn std::error::Error>>
{
    let address =
        start_server(Router::new().route("/v1/responses", post(truncated_stream))).await?;
    let client = OpenAiModel::new(
        ModelConfig::new(format!("http://{address}"))
            .with_default_model("test")
            .with_api(ModelApi::Responses),
    )?;
    let error = client
        .complete_text("x")
        .await
        .expect_err("a stream without response.completed must fail");
    assert!(matches!(
        error,
        ModelError::InvalidResponse(message) if message.contains("terminal event")
    ));
    Ok(())
}

async fn healthy_but_long_running_stream() -> Response {
    delayed_sse(vec![
        (
            Duration::from_millis(35),
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}\n\n",
        ),
        (
            Duration::from_millis(35),
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"b\"}\n\n",
        ),
        (
            Duration::from_millis(35),
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"c\"}\n\n",
        ),
        (
            Duration::from_millis(35),
            "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n",
        ),
    ])
}

async fn silent_stream() -> Response {
    delayed_sse(vec![(
        Duration::from_millis(200),
        "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n",
    )])
}

async fn truncated_stream() -> Response {
    delayed_sse(vec![(
        Duration::ZERO,
        "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"partial\"}\n\n",
    )])
}

async fn delayed_headers() -> Response {
    tokio::time::sleep(Duration::from_millis(200)).await;
    delayed_sse(vec![(
        Duration::ZERO,
        "event: response.completed\ndata: {\"type\":\"response.completed\"}\n\n",
    )])
}

fn delayed_sse(chunks: Vec<(Duration, &'static str)>) -> Response {
    let stream = stream! {
        for (delay, chunk) in chunks {
            tokio::time::sleep(delay).await;
            yield Ok::<Bytes, Infallible>(Bytes::from_static(chunk.as_bytes()));
        }
    };
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    response
}
