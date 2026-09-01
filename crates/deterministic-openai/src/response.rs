//! HTTP response formatting for deterministic fixture behavior.

use std::convert::Infallible;
use std::time::Duration;

use async_stream::stream;
use axum::body::{Body, Bytes};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::sse::{Event, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::contract::ContractError;

pub(super) fn delayed_json_response(body: Value, delay_ms: u64) -> Response {
    let body = body.to_string();
    let stream = stream! {
        if delay_ms > 0 {
            tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        }
        yield Ok::<Bytes, Infallible>(Bytes::from(body));
    };
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    response
}

pub(super) fn invalid_request(error: ContractError) -> Response {
    fixture_error(
        StatusCode::BAD_REQUEST,
        error.code,
        &error.message,
        "invalid_request_error",
    )
}

pub(super) fn invalid_directive(error: ContractError) -> Response {
    fixture_error(
        StatusCode::BAD_REQUEST,
        error.code,
        &error.message,
        "mock_directive_error",
    )
}

pub(super) fn fixture_error(
    status: StatusCode,
    code: &str,
    message: &str,
    error_type: &str,
) -> Response {
    (
        status,
        Json(json!({
            "error": { "message": message, "type": error_type, "code": code }
        })),
    )
        .into_response()
}

pub(super) struct FixtureSseEvent {
    event: Option<&'static str>,
    data: String,
    delay: bool,
}

impl FixtureSseEvent {
    pub(super) fn json(event: Option<&'static str>, data: Value, delay: bool) -> Self {
        Self {
            event,
            data: data.to_string(),
            delay,
        }
    }

    pub(super) fn raw(event: Option<&'static str>, data: impl Into<String>, delay: bool) -> Self {
        Self {
            event,
            data: data.into(),
            delay,
        }
    }
}

pub(super) fn responses_event(
    event: &'static str,
    sequence_number: u64,
    data: Value,
    delay: bool,
) -> FixtureSseEvent {
    let data = match data {
        Value::Object(mut data) => {
            data.insert("type".to_owned(), Value::String(event.to_owned()));
            data.insert(
                "sequence_number".to_owned(),
                Value::Number(sequence_number.into()),
            );
            Value::Object(data)
        }
        _ => json!({ "type": event, "sequence_number": sequence_number }),
    };
    FixtureSseEvent::json(Some(event), data, delay)
}

pub(super) fn sse_response(events: Vec<FixtureSseEvent>, delay_ms: u64) -> Response {
    let stream = stream! {
        for event in events {
            if event.delay && delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            let event = if let Some(event_type) = event.event {
                Event::default().event(event_type).data(event.data)
            } else {
                Event::default().data(event.data)
            };
            yield Ok::<Event, Infallible>(event);
        }
    };
    Sse::new(stream).into_response()
}
