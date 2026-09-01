use serde_json::json;

use super::request_json;
use crate::{
    ModelApi, ModelError, ModelMessage, ModelRequest, ModelSelection, ModelToolCall,
    ModelToolDefinition, ReasoningEffort,
};

#[test]
fn serializes_exact_chat_and_stateless_responses_function_requests() -> Result<(), ModelError> {
    let request = ModelRequest {
        messages: vec![
            ModelMessage::User {
                content: "Find weather".to_owned(),
            },
            ModelMessage::Assistant {
                content: None,
                tool_calls: vec![ModelToolCall {
                    id: "call-1".to_owned(),
                    name: "weather".to_owned(),
                    arguments: r#"{"city":"Seattle"}"#.to_owned(),
                }],
            },
            ModelMessage::Tool {
                tool_call_id: "call-1".to_owned(),
                content: r#"{"temperature":12}"#.to_owned(),
            },
        ],
        tools: vec![ModelToolDefinition::new(
            "weather",
            "Looks up weather.",
            json!({"type": "object"}),
        )],
        model_selection: None,
    };
    assert_eq!(
        request_json(ModelApi::ChatCompletions, "model", &request)?,
        json!({
            "model": "model",
            "messages": [
                {"role": "user", "content": "Find weather"},
                {"role": "assistant", "content": null, "tool_calls": [{
                    "id": "call-1", "type": "function",
                    "function": {"name": "weather", "arguments": r#"{"city":"Seattle"}"#}
                }]},
                {"role": "tool", "tool_call_id": "call-1", "content": r#"{"temperature":12}"#}
            ],
            "tools": [{"type": "function", "function": {
                "name": "weather", "description": "Looks up weather.", "parameters": {"type": "object"}
            }}],
            "parallel_tool_calls": false,
            "stream": true,
        })
    );
    assert_eq!(
        request_json(ModelApi::Responses, "model", &request)?,
        json!({
            "model": "model",
            "input": [
                {"role": "user", "content": "Find weather"},
                {"type": "function_call", "call_id": "call-1", "name": "weather", "arguments": r#"{"city":"Seattle"}"#},
                {"type": "function_call_output", "call_id": "call-1", "output": r#"{"temperature":12}"#}
            ],
            "tools": [{"type": "function", "name": "weather", "description": "Looks up weather.", "parameters": {"type": "object"}}],
            "parallel_tool_calls": false,
            "stream": true,
        })
    );
    Ok(())
}

#[test]
fn serializes_selected_model_and_reasoning_effort_for_each_api() -> Result<(), ModelError> {
    let request = ModelRequest::text("hello").with_model_selection(
        ModelSelection::new("selected-model").with_reasoning_effort(ReasoningEffort::Medium),
    );
    let chat = request_json(ModelApi::ChatCompletions, "configured-model", &request)?;
    assert_eq!(chat["model"], "selected-model");
    assert_eq!(chat["reasoning_effort"], "medium");
    assert!(chat.get("reasoning").is_none());

    let responses = request_json(ModelApi::Responses, "configured-model", &request)?;
    assert_eq!(responses["model"], "selected-model");
    assert_eq!(
        responses.pointer("/reasoning/effort"),
        Some(&json!("medium"))
    );
    assert!(responses.get("reasoning_effort").is_none());
    Ok(())
}

#[test]
fn omits_reasoning_and_uses_configured_model_without_a_selection() -> Result<(), ModelError> {
    let request = ModelRequest::text("hello");
    for api in [ModelApi::Responses, ModelApi::ChatCompletions] {
        let value = request_json(api, "configured-model", &request)?;
        assert_eq!(value["model"], "configured-model");
        assert!(value.get("reasoning").is_none());
        assert!(value.get("reasoning_effort").is_none());
    }

    let selected =
        ModelRequest::text("hello").with_model_selection(ModelSelection::new("selected"));
    for api in [ModelApi::Responses, ModelApi::ChatCompletions] {
        let value = request_json(api, "configured-model", &selected)?;
        assert_eq!(value["model"], "selected");
        assert!(value.get("reasoning").is_none());
        assert!(value.get("reasoning_effort").is_none());
    }
    Ok(())
}

#[test]
fn rejects_an_empty_selected_model_before_serialization() {
    let request = ModelRequest::text("hello").with_model_selection(ModelSelection::new("  "));
    assert!(matches!(
        request_json(ModelApi::Responses, "configured-model", &request),
        Err(ModelError::InvalidModelSelection { .. })
    ));
}
