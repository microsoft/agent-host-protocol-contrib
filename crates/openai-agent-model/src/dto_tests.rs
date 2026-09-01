use serde_json::json;

use super::request_json;
use crate::{ModelApi, ModelMessage, ModelRequest, ModelToolCall, ModelToolDefinition};

#[test]
fn serializes_exact_chat_and_stateless_responses_function_requests() {
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
    };
    assert_eq!(
        request_json(ModelApi::ChatCompletions, "model", &request),
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
        request_json(ModelApi::Responses, "model", &request),
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
}
