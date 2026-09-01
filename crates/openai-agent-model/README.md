# openai-agent-model

`openai-agent-model` is a small, reusable Rust adapter for streaming
OpenAI-compatible model endpoints. Its library name is `openai_agent_model`.
It accepts a stateful application conversation as `ModelRequest`, but submits
the complete request on every call, so Responses API tool rounds remain
stateless.

```rust,no_run
use openai_agent_model::{ModelConfig, OpenAiModel};

let model = OpenAiModel::new(ModelConfig::new(
    "https://api.openai.com/v1",
    "gpt-4.1-mini",
))?;
# Ok::<(), openai_agent_model::ModelError>(())
```

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
The adapter emits text and function-argument deltas, validates terminal
events, disables parallel tool calls, and cancels the response body when the
returned `ModelEventStream` is dropped.

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
