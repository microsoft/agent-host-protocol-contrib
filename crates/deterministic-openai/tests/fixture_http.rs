use deterministic_openai::{ObservedApi, RunningFixture};
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio::time::{timeout, Duration};

const INVALID_REQUEST: &str = "AHP_MOCK_INVALID_REQUEST";
const UPSTREAM: &str = "AHP_TEST_UPSTREAM";

#[tokio::test]
async fn v1_serves_raw_chat_and_responses_sse_with_full_done_snapshots(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let prompt = v1_prompt("aé🙂", 3, 0);

    let chat_endpoint = chat_endpoint(&fixture);
    let chat: Value = client
        .post(&chat_endpoint)
        .json(&chat_request(&prompt, false))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(chat["id"], "ahp-test-v1");
    assert_eq!(
        chat.pointer("/choices/0/message/content"),
        Some(&json!("aé🙂"))
    );
    assert_eq!(
        chat.pointer("/choices/0/finish_reason"),
        Some(&json!("stop"))
    );

    let response: Value = client
        .post(responses_endpoint(&fixture))
        .json(&responses_request(&prompt, false))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        response.pointer("/output/0/content/0/text"),
        Some(&json!("aé🙂"))
    );

    let chat_events = stream(&client, &chat_endpoint, chat_request(&prompt, true)).await?;
    assert_eq!(text_deltas(&chat_events), "aé🙂");
    assert_eq!(
        chat_events
            .iter()
            .rev()
            .nth(1)
            .and_then(|event| event.pointer("/choices/0/finish_reason")),
        Some(&json!("stop"))
    );
    assert_eq!(chat_events.last(), Some(&json!("[DONE]")));

    let response_events = stream(
        &client,
        &responses_endpoint(&fixture),
        responses_request(&prompt, true),
    )
    .await?;
    assert_eq!(text_deltas(&response_events), "aé🙂");
    assert_sequence_numbers(&response_events);
    assert_eq!(
        event(&response_events, "response.output_text.done")?["text"],
        json!("aé🙂")
    );
    assert_eq!(
        event(&response_events, "response.output_text.done")?["logprobs"],
        json!([])
    );
    assert_eq!(
        event(&response_events, "response.content_part.done")?.pointer("/part/text"),
        Some(&json!("aé🙂"))
    );
    assert_eq!(
        event(&response_events, "response.output_item.done")?.pointer("/item/content/0/text"),
        Some(&json!("aé🙂"))
    );
    assert_eq!(
        event(&response_events, "response.completed")?.pointer("/response/output/0/content/0/text"),
        Some(&json!("aé🙂"))
    );

    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn v3_repeats_prior_assistant_text_for_both_apis() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let prompt = "[AHP-TEST v3]\nRepeat the previous assistant response exactly.\n[/AHP-TEST]";

    let chat: Value = client
        .post(chat_endpoint(&fixture))
        .json(&json!({
            "model": "fixture-model",
            "messages": [
                { "role": "user", "content": "first" },
                { "role": "assistant", "content": "remembered" },
                { "role": "user", "content": prompt }
            ]
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        chat.pointer("/choices/0/message/content"),
        Some(&json!("remembered"))
    );

    let response: Value = client
        .post(responses_endpoint(&fixture))
        .json(&json!({
            "model": "fixture-model",
            "input": [
                { "role": "user", "content": "first" },
                { "role": "assistant", "content": "remembered" },
                { "role": "user", "content": prompt }
            ]
        }))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(
        response.pointer("/output/0/content/0/text"),
        Some(&json!("remembered"))
    );

    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn v2_progresses_tools_and_streams_arguments_for_both_apis(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let prompt = concat!(
        "[AHP-TEST v2]\n",
        "Call tool weather with JSON: {\"city\":\"Seattle\",\"days\":2}\n",
        "Call tool summarize with JSON: {\"forecast\":\"rain\"}\n",
        "Respond exactly after tools with JSON: \"Rainy.\".\n",
        "[/AHP-TEST]"
    );
    let chat_endpoint = chat_endpoint(&fixture);

    let chat_events = stream(
        &client,
        &chat_endpoint,
        chat_v2_request(prompt, vec![], chat_tools(&["weather", "summarize"]), true),
    )
    .await?;
    assert_eq!(
        chat_events[0].pointer("/choices/0/delta/tool_calls/0/id"),
        Some(&json!("ahp-test-tool-1"))
    );
    assert_eq!(
        chat_tool_arguments(&chat_events),
        r#"{"city":"Seattle","days":2}"#
    );
    assert_eq!(
        chat_events
            .iter()
            .rev()
            .nth(1)
            .and_then(|event| event.pointer("/choices/0/finish_reason")),
        Some(&json!("tool_calls"))
    );

    let second_chat = request_json(
        &client,
        &chat_endpoint,
        chat_v2_request(
            prompt,
            vec![
                json!({"role": "user", "content": prompt}),
                chat_function_call(
                    "ahp-test-tool-1",
                    "weather",
                    r#"{"city":"Seattle","days":2}"#,
                ),
                json!({"role": "tool", "tool_call_id": "ahp-test-tool-1", "content": "rain"}),
            ],
            chat_tools(&["weather", "summarize"]),
            false,
        ),
    )
    .await?;
    assert_eq!(
        second_chat.pointer("/choices/0/message/tool_calls/0/id"),
        Some(&json!("ahp-test-tool-2"))
    );

    let responses_endpoint = responses_endpoint(&fixture);
    let first_response = stream(
        &client,
        &responses_endpoint,
        responses_v2_request(
            prompt,
            vec![],
            responses_tools(&["weather", "summarize"]),
            true,
        ),
    )
    .await?;
    assert_eq!(
        event(&first_response, "response.output_item.added")?.pointer("/item/call_id"),
        Some(&json!("ahp-test-tool-1"))
    );
    assert_eq!(
        function_argument_deltas(&first_response),
        r#"{"city":"Seattle","days":2}"#
    );
    assert_eq!(
        event(&first_response, "response.function_call_arguments.done")?["arguments"],
        json!(r#"{"city":"Seattle","days":2}"#)
    );
    assert_eq!(
        event(&first_response, "response.function_call_arguments.done")?["name"],
        json!("weather")
    );

    let second_response = request_json(
        &client,
        &responses_endpoint,
        responses_v2_request(
            prompt,
            responses_after_first(prompt),
            responses_tools(&["weather", "summarize"]),
            false,
        ),
    )
    .await?;
    assert_eq!(
        second_response.pointer("/output/0/call_id"),
        Some(&json!("ahp-test-tool-2"))
    );

    let final_events = stream(
        &client,
        &responses_endpoint,
        responses_v2_request(
            prompt,
            responses_after_second(prompt),
            responses_tools(&["weather", "summarize"]),
            true,
        ),
    )
    .await?;
    assert_eq!(text_deltas(&final_events), "Rainy.");
    assert_eq!(
        event(&final_events, "response.output_text.done")?["text"],
        "Rainy."
    );

    fixture.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn validates_errors_and_chat_only_responses_404() -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let endpoint = chat_endpoint(&fixture);

    assert_error(
        client
            .post(&endpoint)
            .json(&json!({"model": "fixture", "messages": [], "stream": false}))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        INVALID_REQUEST,
        "invalid_request_error",
    )
    .await?;
    assert_error(
        client
            .post(&endpoint)
            .json(&chat_request(
                "[AHP-TEST v1]\nRespond exactly with JSON: \"x\"",
                false,
            ))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "AHP_TEST_INVALID_DIRECTIVE",
        "mock_directive_error",
    )
    .await?;
    assert_error(
        client
            .post(&endpoint)
            .json(&chat_request(
                "[AHP-TEST v4]\nRespond exactly with JSON: \"x\"\n[/AHP-TEST]",
                false,
            ))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "AHP_TEST_UNSUPPORTED_VERSION",
        "mock_directive_error",
    )
    .await?;
    assert_error(
        client
            .post(&endpoint)
            .json(&chat_request(
                "[AHP-TEST v1]\nReturn upstream HTTP 503 with JSON message: \"planned\".\n[/AHP-TEST]",
                true,
            ))
            .send()
            .await?,
        StatusCode::SERVICE_UNAVAILABLE,
        UPSTREAM,
        "mock_upstream_error",
    )
    .await?;
    let unknown_tool_prompt = concat!(
        "[AHP-TEST v2]\n",
        "Call tool weather with JSON: {}\n",
        "Respond exactly after tools with JSON: \"x\".\n",
        "[/AHP-TEST]"
    );
    assert_error(
        client
            .post(&endpoint)
            .json(&chat_v2_request(
                unknown_tool_prompt,
                vec![],
                chat_tools(&["not_weather"]),
                false,
            ))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        "AHP_MOCK_UNKNOWN_TOOL",
        "invalid_request_error",
    )
    .await?;
    fixture.shutdown().await?;

    let chat_only = RunningFixture::start_chat_only("127.0.0.1:0".parse()?).await?;
    assert_error(
        client
            .post(responses_endpoint(&chat_only))
            .json(&responses_request("hello", false))
            .send()
            .await?,
        StatusCode::NOT_FOUND,
        INVALID_REQUEST,
        "invalid_request_error",
    )
    .await?;
    let chat = request_json(
        &client,
        &chat_endpoint(&chat_only),
        chat_request("hello", false),
    )
    .await?;
    assert_eq!(
        chat.pointer("/choices/0/message/content"),
        Some(&json!("Deterministic echo: hello"))
    );
    chat_only.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn lists_models_and_observes_selected_model_and_reasoning_effort(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let models: Value = client
        .get(format!("{}/v1/models", fixture.base_url()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(models["object"], "list");
    assert_eq!(
        models["data"],
        json!([
            {"id": "fixture-model", "owned_by": "ahp-test"},
            {"id": "fixture-fast", "owned_by": "ahp-test"},
            {"id": "fixture-reasoning", "owned_by": "ahp-test"},
        ])
    );

    let mut chat = chat_request("selected chat", false);
    chat["model"] = json!("fixture-fast");
    chat["reasoning_effort"] = json!("xhigh");
    request_json(&client, &chat_endpoint(&fixture), chat).await?;

    let mut responses = responses_request("selected responses", false);
    responses["model"] = json!("fixture-reasoning");
    responses["reasoning"] = json!({"effort": "medium"});
    request_json(&client, &responses_endpoint(&fixture), responses).await?;

    assert_eq!(
        fixture.requests(),
        vec![
            deterministic_openai::ObservedRequest {
                api: ObservedApi::ChatCompletions,
                model: "fixture-fast".to_owned(),
                reasoning_effort: Some("xhigh".to_owned()),
            },
            deterministic_openai::ObservedRequest {
                api: ObservedApi::Responses,
                model: "fixture-reasoning".to_owned(),
                reasoning_effort: Some("medium".to_owned()),
            },
        ]
    );

    assert_error(
        client
            .post(chat_endpoint(&fixture))
            .json(&json!({
                "model": "fixture-model",
                "messages": [{"role": "user", "content": "invalid"}],
                "reasoning_effort": false,
            }))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        INVALID_REQUEST,
        "invalid_request_error",
    )
    .await?;
    assert_error(
        client
            .post(responses_endpoint(&fixture))
            .json(&json!({
                "model": "fixture-model",
                "input": [{"role": "user", "content": "invalid"}],
                "reasoning": {"effort": "unsupported"},
            }))
            .send()
            .await?,
        StatusCode::BAD_REQUEST,
        INVALID_REQUEST,
        "invalid_request_error",
    )
    .await?;
    fixture.shutdown().await?;

    let chat_only = RunningFixture::start_chat_only("127.0.0.1:0".parse()?).await?;
    let models: Value = client
        .get(format!("{}/v1/models", chat_only.base_url()))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    assert_eq!(models["data"].as_array().map(Vec::len), Some(3));
    chat_only.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn applies_delays_and_remains_usable_after_disconnect(
) -> Result<(), Box<dyn std::error::Error>> {
    let fixture = start().await?;
    let client = reqwest::Client::new();
    let endpoint = chat_endpoint(&fixture);

    let started = std::time::Instant::now();
    let events = stream(
        &client,
        &endpoint,
        chat_request(&v1_prompt("abc", 3, 25), true),
    )
    .await?;
    assert_eq!(text_deltas(&events), "abc");
    assert!(started.elapsed() >= Duration::from_millis(70));

    let delayed = client
        .post(&endpoint)
        .json(&chat_request(&v1_prompt("later", 1, 500), false))
        .send()
        .await?;
    assert_eq!(delayed.status(), StatusCode::OK);
    drop(delayed);
    let chat = timeout(
        Duration::from_secs(1),
        request_json(&client, &endpoint, chat_request("still responsive", false)),
    )
    .await??;
    assert_eq!(
        chat.pointer("/choices/0/message/content"),
        Some(&json!("Deterministic echo: still responsive"))
    );

    fixture.shutdown().await?;
    Ok(())
}

async fn start() -> Result<RunningFixture, Box<dyn std::error::Error>> {
    Ok(RunningFixture::start("127.0.0.1:0".parse()?).await?)
}

fn chat_endpoint(fixture: &RunningFixture) -> String {
    format!("{}/v1/chat/completions", fixture.base_url())
}

fn responses_endpoint(fixture: &RunningFixture) -> String {
    format!("{}/v1/responses", fixture.base_url())
}

fn chat_request(content: &str, stream: bool) -> Value {
    json!({
        "model": "fixture-model",
        "messages": [{"role": "user", "content": content}],
        "stream": stream,
    })
}

fn responses_request(content: &str, stream: bool) -> Value {
    json!({
        "model": "fixture-model",
        "input": [{"role": "user", "content": content}],
        "stream": stream,
    })
}

fn v1_prompt(text: &str, chunks: usize, delay_ms: u64) -> String {
    format!(
        "[AHP-TEST v1]\nRespond exactly with JSON: {text:?}\nStream in exactly {chunks} chunks.\nWait {delay_ms} ms before every output chunk.\n[/AHP-TEST]"
    )
}

fn chat_v2_request(prompt: &str, messages: Vec<Value>, tools: Vec<Value>, stream: bool) -> Value {
    json!({
        "model": "fixture-model",
        "messages": if messages.is_empty() {
            vec![json!({"role": "user", "content": prompt})]
        } else {
            messages
        },
        "tools": tools,
        "stream": stream,
    })
}

fn responses_v2_request(prompt: &str, input: Vec<Value>, tools: Vec<Value>, stream: bool) -> Value {
    json!({
        "model": "fixture-model",
        "input": if input.is_empty() {
            vec![json!({"role": "user", "content": prompt})]
        } else {
            input
        },
        "tools": tools,
        "stream": stream,
        "parallel_tool_calls": false,
    })
}

fn chat_tools(names: &[&str]) -> Vec<Value> {
    names
        .iter()
        .map(|name| {
            json!({"type": "function", "function": {"name": name, "parameters": {"type": "object"}}})
        })
        .collect()
}

fn responses_tools(names: &[&str]) -> Vec<Value> {
    names
        .iter()
        .map(|name| json!({"type": "function", "name": name, "parameters": {"type": "object"}}))
        .collect()
}

fn chat_function_call(id: &str, name: &str, arguments: &str) -> Value {
    json!({
        "role": "assistant",
        "content": null,
        "tool_calls": [{"id": id, "type": "function", "function": {"name": name, "arguments": arguments}}],
    })
}

fn responses_after_first(prompt: &str) -> Vec<Value> {
    vec![
        json!({"role": "user", "content": prompt}),
        json!({"type": "function_call", "call_id": "ahp-test-tool-1", "name": "weather", "arguments": r#"{"city":"Seattle","days":2}"#}),
        json!({"type": "function_call_output", "call_id": "ahp-test-tool-1", "output": "rain"}),
    ]
}

fn responses_after_second(prompt: &str) -> Vec<Value> {
    let mut input = responses_after_first(prompt);
    input.extend([
        json!({"type": "function_call", "call_id": "ahp-test-tool-2", "name": "summarize", "arguments": r#"{"forecast":"rain"}"#}),
        json!({"type": "function_call_output", "call_id": "ahp-test-tool-2", "output": "Rainy."}),
    ]);
    input
}

async fn request_json(
    client: &reqwest::Client,
    endpoint: &str,
    body: Value,
) -> Result<Value, reqwest::Error> {
    client
        .post(endpoint)
        .json(&body)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

async fn stream(
    client: &reqwest::Client,
    endpoint: &str,
    body: Value,
) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .await?
        .error_for_status()?;
    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream")
    );
    let mut events = Vec::new();
    for data in response
        .text()
        .await?
        .lines()
        .filter_map(|line| line.strip_prefix("data: "))
    {
        events.push(if data == "[DONE]" {
            json!("[DONE]")
        } else {
            serde_json::from_str(data)?
        });
    }
    Ok(events)
}

fn text_deltas(events: &[Value]) -> String {
    events
        .iter()
        .filter(|event| {
            event["type"] == "response.output_text.delta"
                || event.pointer("/choices/0/delta/content").is_some()
        })
        .filter_map(|event| {
            event["delta"].as_str().or_else(|| {
                event
                    .pointer("/choices/0/delta/content")
                    .and_then(Value::as_str)
            })
        })
        .collect()
}

fn chat_tool_arguments(events: &[Value]) -> String {
    events
        .iter()
        .filter_map(|event| event.pointer("/choices/0/delta/tool_calls/0/function/arguments"))
        .filter_map(Value::as_str)
        .collect()
}

fn function_argument_deltas(events: &[Value]) -> String {
    events
        .iter()
        .filter(|event| event["type"] == "response.function_call_arguments.delta")
        .filter_map(|event| event["delta"].as_str())
        .collect()
}

fn event<'a>(events: &'a [Value], event_type: &str) -> Result<&'a Value, &'static str> {
    events
        .iter()
        .find(|event| event["type"] == event_type)
        .ok_or("fixture event must be present")
}

fn assert_sequence_numbers(events: &[Value]) {
    for (index, event) in events.iter().enumerate() {
        assert_eq!(event["sequence_number"], json!(index as u64));
    }
}

async fn assert_error(
    response: reqwest::Response,
    status: StatusCode,
    code: &str,
    error_type: &str,
) -> Result<(), reqwest::Error> {
    assert_eq!(response.status(), status);
    let body: Value = response.json().await?;
    assert_eq!(body.pointer("/error/code"), Some(&json!(code)));
    assert_eq!(body.pointer("/error/type"), Some(&json!(error_type)));
    Ok(())
}
