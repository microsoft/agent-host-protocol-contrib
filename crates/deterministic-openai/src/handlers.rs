//! Endpoint handlers and deterministic v1/v2/v3/v4 response selection.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde_json::{json, Value};

use super::chat_stream;
use super::contract::{code, split_chunks, ContractError};
use super::nonstream;
use super::request::FixtureRequest;
use super::response::{fixture_error, invalid_directive, invalid_request};
use super::responses_stream;
use super::v1::FixtureSpec;
use super::v2::{self, V2Action, V2FixtureSpec};
use super::{v3, v4};
use super::{
    FixtureState, ObservedApi, ObservedRequest, CHAT_COMPLETION_ID, RESPONSES_ID,
    V2_CHAT_COMPLETION_ID, V2_RESPONSES_ID,
};

pub(super) async fn chat_completions(
    State(state): State<FixtureState>,
    request: Result<Json<Value>, JsonRejection>,
) -> Response {
    let request = match parse_request(request, FixtureRequest::parse_chat) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    observe_request(&state, ObservedApi::ChatCompletions, &request);
    if v4::has_marker(&request.last_user_content) {
        return v2_chat_response(request);
    }
    if v3::has_marker(&request.last_user_content) {
        let response = match v3::previous_response(&request) {
            Ok(response) => response,
            Err(error) => return contract_error(error),
        };
        return if request.stream {
            chat_stream::text(CHAT_COMPLETION_ID, vec![response], 0)
        } else {
            nonstream::chat_text(CHAT_COMPLETION_ID, response, 0)
        };
    }
    if v2::has_marker(&request.last_user_content) {
        return v2_chat_response(request);
    }
    let spec = match FixtureSpec::parse(&request.last_user_content) {
        Ok(spec) => spec,
        Err(error) => return invalid_directive(error),
    };
    if let Some(error) = spec.upstream_error {
        return upstream_error(error.status, &error.message);
    }
    if request.stream {
        return chat_stream::text(
            CHAT_COMPLETION_ID,
            split_chunks(&spec.response, spec.chunks),
            spec.delay_ms,
        );
    }
    nonstream::chat_text(CHAT_COMPLETION_ID, spec.response, spec.delay_ms)
}

pub(super) async fn responses(
    State(state): State<FixtureState>,
    request: Result<Json<Value>, JsonRejection>,
) -> Response {
    if !state.responses_enabled {
        return fixture_error(
            StatusCode::NOT_FOUND,
            code::INVALID_REQUEST,
            "the deterministic fixture was started without Responses API support",
            "invalid_request_error",
        );
    }
    let request = match parse_request(request, FixtureRequest::parse_responses) {
        Ok(request) => request,
        Err(response) => return *response,
    };
    observe_request(&state, ObservedApi::Responses, &request);
    if v4::has_marker(&request.last_user_content) {
        return v2_responses_response(request);
    }
    if v3::has_marker(&request.last_user_content) {
        let response = match v3::previous_response(&request) {
            Ok(response) => response,
            Err(error) => return contract_error(error),
        };
        return if request.stream {
            responses_stream::text(RESPONSES_ID, vec![response], 0)
        } else {
            nonstream::responses_text(RESPONSES_ID, response, 0)
        };
    }
    if v2::has_marker(&request.last_user_content) {
        return v2_responses_response(request);
    }
    let spec = match FixtureSpec::parse(&request.last_user_content) {
        Ok(spec) => spec,
        Err(error) => return invalid_directive(error),
    };
    if let Some(error) = spec.upstream_error {
        return upstream_error(error.status, &error.message);
    }
    if request.stream {
        return responses_stream::text(
            RESPONSES_ID,
            split_chunks(&spec.response, spec.chunks),
            spec.delay_ms,
        );
    }
    nonstream::responses_text(RESPONSES_ID, spec.response, spec.delay_ms)
}

pub(super) async fn models() -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": [
            {"id": "fixture-model", "owned_by": "ahp-test"},
            {"id": "fixture-fast", "owned_by": "ahp-test"},
            {"id": "fixture-reasoning", "owned_by": "ahp-test"},
        ],
    }))
}

fn v2_chat_response(request: FixtureRequest) -> Response {
    let (action, delay_ms) = match v2_action(&request) {
        Ok(action) => action,
        Err(response) => return *response,
    };
    match action {
        V2Action::ToolCall {
            id,
            name,
            arguments,
        } if request.stream => chat_stream::tool_call(
            V2_CHAT_COMPLETION_ID,
            0,
            id,
            name,
            split_for_incremental_output(&arguments),
            delay_ms,
        ),
        V2Action::FinalText(content) if request.stream => chat_stream::text(
            V2_CHAT_COMPLETION_ID,
            split_for_incremental_output(&content),
            delay_ms,
        ),
        action => nonstream::v2_chat(action, delay_ms),
    }
}

fn v2_responses_response(request: FixtureRequest) -> Response {
    let (action, delay_ms) = match v2_action(&request) {
        Ok(action) => action,
        Err(response) => return *response,
    };
    match action {
        V2Action::ToolCall {
            id,
            name,
            arguments,
        } if request.stream => responses_stream::tool_call(
            V2_RESPONSES_ID,
            0,
            id,
            name,
            arguments.clone(),
            split_for_incremental_output(&arguments),
            delay_ms,
        ),
        V2Action::FinalText(content) if request.stream => responses_stream::text(
            V2_RESPONSES_ID,
            split_for_incremental_output(&content),
            delay_ms,
        ),
        action => nonstream::v2_responses(action, delay_ms),
    }
}

fn v2_action(request: &FixtureRequest) -> Result<(V2Action, u64), Box<Response>> {
    if v4::has_marker(&request.last_user_content) {
        return v4::next_action(request)
            .map(|action| (action, 0))
            .map_err(|error| Box::new(contract_error(error)));
    }
    let spec = V2FixtureSpec::parse(&request.last_user_content)
        .map_err(|error| Box::new(invalid_directive(error)))?;
    if let Err(error) = spec.validate_tools(&request.tool_names) {
        return Err(if error.code == code::UNKNOWN_TOOL {
            Box::new(invalid_request(error))
        } else {
            Box::new(invalid_directive(error))
        });
    }
    spec.next_action(request)
        .map(|action| (action, spec.delay_ms))
        .map_err(|error| Box::new(invalid_request(error)))
}

fn parse_request(
    request: Result<Json<Value>, JsonRejection>,
    parse: fn(Value) -> Result<FixtureRequest, ContractError>,
) -> Result<FixtureRequest, Box<Response>> {
    match request {
        Ok(Json(request)) => parse(request).map_err(|error| Box::new(invalid_request(error))),
        Err(_) => Err(Box::new(fixture_error(
            StatusCode::BAD_REQUEST,
            code::INVALID_REQUEST,
            "request body must be valid JSON",
            "invalid_request_error",
        ))),
    }
}

fn observe_request(state: &FixtureState, api: ObservedApi, request: &FixtureRequest) {
    state.record_request(ObservedRequest {
        api,
        model: request.model.clone(),
        reasoning_effort: request.reasoning_effort.clone(),
    });
}

fn split_for_incremental_output(value: &str) -> Vec<String> {
    split_chunks(value, value.chars().count().clamp(1, 2))
}

fn upstream_error(status: StatusCode, message: &str) -> Response {
    fixture_error(status, code::UPSTREAM, message, "mock_upstream_error")
}

fn contract_error(error: ContractError) -> Response {
    if matches!(error.code, code::INVALID_REQUEST | code::NO_SUITABLE_TOOL) {
        invalid_request(error)
    } else {
        invalid_directive(error)
    }
}
