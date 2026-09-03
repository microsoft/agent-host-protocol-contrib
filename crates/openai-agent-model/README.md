# openai-agent-model

`openai-agent-model` is a small, reusable Rust adapter for streaming
OpenAI-compatible model endpoints. Its library name is `openai_agent_model`.
It accepts a complete application conversation as `ModelRequest`. By default it
submits that full request on every call. With
`ModelConfig::with_response_chaining(true)`, a `ResponseContinuation` sends only
the new Responses input plus `previous_response_id`; the complete messages stay
available if automatic API selection falls back to Chat Completions.

```rust,no_run
use openai_agent_model::{ModelConfig, OpenAiModel};

let model = OpenAiModel::new(ModelConfig::new(
    "https://api.openai.com/v1",
    "gpt-4.1-mini",
))?;
# Ok::<(), openai_agent_model::ModelError>(())
```

## Model discovery and request selection

`OpenAiModel::list_models()` requests `GET /v1/models` using the configured
authentication and timeouts. It accepts a host root, `/v1`, or a full
`/responses` or `/chat/completions` URL; returned `ModelDescriptor` values are
sorted and deduplicated by ID. `ModelError::ModelsUnsupported` identifies only
404, 405, and 501, so a host can explicitly retain its configured default;
authentication, rate-limit, network, malformed-response, and other upstream
failures remain errors.

Use `ModelRequest::with_model_selection(ModelSelection::new("model"))` to
override the configured model for one request. Add
`ReasoningEffort::{None, Minimal, Low, Medium, High, XHigh, Max}` with
`with_reasoning_effort`; it maps to `reasoning.effort` for Responses and
`reasoning_effort` for Chat Completions. Without a selection, the configured
model is used and no reasoning field is sent.

## API selection

`ModelApi::Auto` is the default. It tries `/v1/responses` first and shares the
successful selection across `OpenAiModel` clones. It falls back to
`/v1/chat/completions` only when that *initial* Responses request returns 404,
405, or 501. Authentication, rate-limit, server, request, malformed-stream,
and mid-stream failures never cause fallback.

Select an API explicitly with `ModelConfig::with_api` or
`OpenAiModel::new_with_api`. A complete endpoint URL ending in `/responses` or
`/chat/completions` implies that API and cannot probe the other endpoint.

Both protocols use real SSE (`stream: true` and `Accept: text/event-stream`).
The adapter emits the Responses response ID, text, and function-argument
deltas, validates terminal events, disables parallel tool calls, and cancels
the response body when the returned `ModelEventStream` is dropped.

## Timeouts

`StreamTimeouts::default()` uses a 10-second `connect` timeout and a
75-second `idle` timeout. `connect` bounds both connection establishment and
the initial response-header wait. It is the only timeout configured on
`reqwest::ClientBuilder`; there is deliberately **no total request or body
deadline**.

After headers, each wait for the next body chunk is limited by `idle`.
Any non-empty bytes received from the HTTP body reset the deadline, including
SSE comments, so proxies that send byte-level keepalives keep the stream
alive. Empty chunks do not reset it. A silent stream returns
`ModelError::IdleTimeout`; a healthy completion may run indefinitely while it
continues to make progress within each idle interval.

**Audience:** host authors adapting an OpenAI-compatible streaming endpoint.
Start at `src/lib.rs`, then read `client.rs`, `responses.rs`, `chat.rs`, and
`sse.rs`. Next, see the [advanced host](../../samples/rust/advanced-workspace-host/README.md)
for configuration or the [deterministic fixture](../deterministic-openai/README.md)
for controllable test streams.
