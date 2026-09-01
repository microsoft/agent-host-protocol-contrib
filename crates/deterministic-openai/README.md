# deterministic-openai-fixture

`deterministic-openai-fixture` is a reusable, deterministic HTTP test fixture.
It is **not** an OpenAI implementation and must not be used as one.

It exposes a deliberately small OpenAI-compatible surface for adapter and
integration tests:

```rust
use deterministic_openai::RunningFixture;

let fixture = RunningFixture::start("127.0.0.1:0".parse()?).await?;
let base_url = fixture.base_url();
// Configure a test client with base_url.
fixture.shutdown().await?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Build or test it without the parent workspace:

```powershell
cargo test --manifest-path crates\deterministic-openai\Cargo.toml
```

## API

`RunningFixture::start(bind)` serves both endpoints:

* `POST /v1/chat/completions`
* `POST /v1/responses`
* `GET /v1/models`

`RunningFixture::start_chat_only(bind)` serves Chat Completions and returns a
JSON `404` from Responses; it still serves the model list. The model list uses
the OpenAI list shape and advertises `fixture-model`, `fixture-fast`, and
`fixture-reasoning`, all owned by `ahp-test`. `base_url()` returns the
configured client root and `shutdown(self)` stops the listener.

Both endpoint parsers require a non-empty string `model`, a non-empty input
array (`messages` for Chat, `input` for Responses), and a boolean `stream` if
present. They validate user content, tool definitions, function-call results,
and tool call IDs used in the v2 state machine. Invalid request data returns
`400`, type `invalid_request_error`, and code `AHP_MOCK_INVALID_REQUEST`.

Unknown v2 tools return `AHP_MOCK_UNKNOWN_TOOL`. Malformed directives return
type `mock_directive_error` and code `AHP_TEST_INVALID_DIRECTIVE`; an unsupported
directive version returns `AHP_TEST_UNSUPPORTED_VERSION`. The upstream-error
directive returns its requested 4xx/5xx status, type `mock_upstream_error`, and
code `AHP_TEST_UPSTREAM`.

Requests may use any non-empty model ID. Optional reasoning settings are
validated as OpenAI wire values: Responses accepts
`"reasoning":{"effort":"..."}` and Chat Completions accepts a top-level
`"reasoning_effort":"..."`, where the effort is one of `none`, `minimal`,
`low`, `medium`, `high`, `xhigh`, or `max`. The selected ID never changes the
fixture's v1/v2 behavior. `RunningFixture::requests()` returns a bounded,
concurrency-safe snapshot of successful requests as `ObservedRequest` values
containing `api`, `model`, and `reasoning_effort`.

## Immutable directive contracts

With no complete marker block, the fixture returns `Deterministic echo: ` plus
the last user message. A directive marker must be on a complete line.

### v1 text or error

```text
[AHP-TEST v1]
Respond exactly with JSON: "text"
Stream in exactly 2 chunks.
Wait 10 ms before every output chunk.
[/AHP-TEST]
```

`Respond exactly with JSON` is required for success. The stream and wait lines
are optional. JSON must be a complete JSON string literal. Chunk counts are
1..=1024 and cannot exceed the text's Unicode-scalar length. Alternatively,
the sole success directive can be replaced with:

```text
Return upstream HTTP 503 with JSON message: "planned outage".
```

### v2 sequential tools

```text
[AHP-TEST v2]
Call tool weather with JSON: {"city":"Seattle"}
Call tool summarize with JSON: {"days":2}
Respond exactly after tools with JSON: "Rain expected.".
Wait 10 ms before every output chunk.
[/AHP-TEST]
```

v2 requires at least one `Call tool` directive, tools with matching names in
the request, and a final response directive. Calls are returned one at a time
with IDs `ahp-test-tool-1`, `ahp-test-tool-2`, and so on. Each request must
include completed results after the directive-bearing user message before the
next call or final text is emitted. Tool arguments are canonical JSON with
object keys sorted.

## Streaming behavior

Chat emits deterministic `chat.completion.chunk` text or function-call
argument chunks, a terminal chunk (`stop` or `tool_calls`), then `[DONE]`.

Responses emits event names and matching `type` fields from the Responses
streaming schema, including increasing `sequence_number` fields. Text streams
emit `response.output_text.delta`, followed by:

* `response.output_text.done.text` containing the concatenated text;
* `response.content_part.done.part` containing the full `output_text` part;
* `response.output_item.done.item.content` containing that same full part.

The terminal `response.completed.response` also contains the completed output
item. Function-call streams similarly emit incremental arguments, a complete
`response.function_call_arguments.done`, and a completed output item. Delays
apply before each streamed text or argument delta and once before non-streaming
output.

**Audience:** adapter and integration-test authors who need stable
OpenAI-shaped HTTP/SSE behavior. Start at `src/lib.rs` and use the immutable
v1/v2 contracts above; next read the
[CLI guide](../../tools/deterministic-openai/README.md) or the
[conformance runner](../../tools/ahp-conformance/README.md).
