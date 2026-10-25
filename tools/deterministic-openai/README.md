# Deterministic OpenAI fixture

Run a local OpenAI-compatible endpoint:

```powershell
cargo run -p deterministic-openai -- --bind 127.0.0.1:8899
```

```bash
cargo run -p deterministic-openai -- --bind 127.0.0.1:8899
```

The CLI uses the reusable `deterministic-openai-fixture` library crate
(`deterministic_openai` in Rust), including `RunningFixture` for test stacks.

It exposes `POST /v1/chat/completions` and `POST /v1/responses`. Both require a
non-empty string `model`, stream by default when `stream: true`, and emit
deterministic SSE. Start with `--chat-only` (or
`RunningFixture::start_chat_only`) to make `/v1/responses` return HTTP 404 for
automatic-fallback tests.

Chat requests use `messages`: `user` messages require string `content`;
`assistant` messages require string-or-null `content` and standard
`tool_calls` function objects; `tool` messages require string `tool_call_id`
and `content`. Chat tools use the nested
`{ "type": "function", "function": ... }` shape. Responses requests use
`input`: user/assistant text items use `role`/`content`, prior calls use
`{ "type": "function_call", "call_id", "name", "arguments" }`, and results use
`{ "type": "function_call_output", "call_id", "output" }`. Responses tools are
flattened `{ "type": "function", "name", "description", "parameters" }`.
Other request properties and extension roles do not control directive selection.
Text from extension roles is retained for v4 phrase recall. The last user item
controls fixture behavior.

Use exactly one immutable, complete-line directive block. The directives are
**exact syntax**: preserve their marker, capitalization, punctuation, and JSON
literals. Natural-language text outside the block and tool-result content are
**semantic** model context, not fixture directives, except for the exact
`Remember this phrase: <JSON string>` history lines recognized by v4 recall.

## v1 text responses

`[AHP-TEST v1]` is immutable. It is readable as an ordinary model instruction
while its exact lines are machine parsed:

```text
[AHP-TEST v1]
Respond exactly with JSON: "A stable answer."
Stream in exactly 3 chunks.
Wait 25 ms before every output chunk.
[/AHP-TEST]
```

The response directive is required for success. Its value is a JSON string, so
it preserves newlines, quotes, backslashes, and Unicode exactly. `Stream` and
`Wait` are optional, defaulting to one chunk and zero milliseconds. Chunk
counts are `1..1024` and cannot exceed the response's Unicode-scalar length
(except empty output, which has one chunk). Chunks are balanced by Unicode
scalar, with extra scalars assigned to earlier chunks.

To exercise an endpoint error instead of a completion:

```text
[AHP-TEST v1]
Return upstream HTTP 503 with JSON message: "planned outage".
[/AHP-TEST]
```

The parser rejects unknown, duplicate, malformed, incompatible, multi-block,
and unsupported-version directives as OpenAI-shaped HTTP 400 errors with
`type: "mock_directive_error"` and stable codes
`AHP_TEST_INVALID_DIRECTIVE` or `AHP_TEST_UNSUPPORTED_VERSION`. Invalid
request shapes use `type: "invalid_request_error"` and
`AHP_MOCK_INVALID_REQUEST`. Explicit upstream errors retain their requested
status and return `type: "mock_upstream_error"` with
`AHP_TEST_UPSTREAM`.

For a non-streaming completion, `Wait` delays the single HTTP JSON response
exactly once, regardless of `Stream in exactly N chunks.` For Chat SSE it
delays each content chunk, but never the terminal stop or `[DONE]`. Responses
uses `response.created`, output-item/content lifecycle events,
`response.output_text.delta`, and `response.completed`; it delays only text
deltas. If the client disconnects, the streaming body and its pending delay
are dropped; no later chunks are emitted.

## v2 sequential function tools

`[AHP-TEST v2]` deterministically drives one function call per completion. Its
strict grammar is one or more ordered calls followed by one final response:

```text
[AHP-TEST v2]
Call tool weather with JSON: {"city":"Seattle","days":2}
Call tool summarize_weather with JSON: {"format":"brief"}
Respond exactly after tools with JSON: "Seattle will be rainy.".
Wait 25 ms before every output chunk.
[/AHP-TEST]
```

Each `Call tool` line's name is an OpenAI function name and its value is one
complete JSON object. The fixture serializes its arguments deterministically.
The final value is a JSON string and the directive ends in a period. `Wait` is
optional. v2 does not accept v1 `Stream` or `Return upstream` directives.

On the first request, every named v2 tool must appear in the request's `tools`
array; the fixture returns the first call with id `ahp-test-tool-1`. Preserve
the assistant tool-call message, then append a tool result with that ID to
advance to the next configured call (`ahp-test-tool-2`, and so on). After all
tool results are present, the fixture returns the final text.

With `stream: true`, v2 streams function arguments and final text in balanced
Unicode chunks (two chunks whenever the value has at least two scalars). `Wait`
is applied before each content or argument delta, never lifecycle terminals.
Chat emits ordinary delta chunks, a finishing chunk, and `[DONE]`. Responses
emits `response.created`, function-call/output-item lifecycle events,
argument deltas/done, and `response.completed`. Missing tools return
OpenAI-shaped HTTP 400 `invalid_request_error` with stable code
`AHP_MOCK_UNKNOWN_TOOL`; malformed v2 directives retain
`mock_directive_error` and `AHP_TEST_INVALID_DIRECTIVE`.

## v3 conversation history

```text
[AHP-TEST v3]
Repeat the previous assistant response exactly.
[/AHP-TEST]
```

Returns the previous assistant text before the directive-bearing user message.

## v4 phrase recall and file tools

v4 accepts exactly one of these directives per complete-line block. The same
natural-language prompts work with real models and with the fixture.

To recall the last phrase established by an earlier complete line
`Remember this phrase: <JSON string>` in any input role:

```text
[AHP-TEST v4]
Recall the phrase I asked you to remember and respond with only that phrase.
[/AHP-TEST]
```

The last matching history line before the current user message wins. A missing
phrase returns HTTP 400 `AHP_TEST_INVALID_DIRECTIVE`.

To read or create a file through an available tool:

```text
[AHP-TEST v4]
Read the file at JSON path: "/workspace/input.txt" with an available tool, then respond with only its exact contents.
[/AHP-TEST]
```

```text
[AHP-TEST v4]
Create the file at JSON path: "/workspace/output.txt" containing JSON text: "exact contents\n" with an available tool, then respond with only the word done.
[/AHP-TEST]
```

Paths and contents are JSON strings. The fixture prefers appropriately named
file tools with string path/content properties, otherwise uses a string
`command`/`cmd` shell tool. Read fallback: `cat -- '<path>'`. Write fallback:
`printf '%s' '<text>' > '<path>'`. Values use POSIX single-quote escaping; the
advanced workspace host's `run_shell` schema supports these fallbacks.
No suitable tool returns HTTP 400 `invalid_request_error` with
`AHP_TEST_NO_SUITABLE_TOOL`.

Calls use ID `ahp-test-tool-1` and v2's result placement and ID validation.
After the tool result, reads return the result content verbatim and writes
return `done`. Subsequent requests need not repeat tool definitions. Both APIs
support streaming and non-streaming v4 output. v4 does not support optional
`Wait` or `Stream` directive lines. See the
[immutable library contracts](../../crates/deterministic-openai/README.md#v4-phrase-recall-and-file-tools)
for exact tool/property precedence and history matching.

**Audience:** developers manually running deterministic model behavior. Start
with the v1 text contract, use v2 for sequential tools or v4 for recall and file
operations, then run the [TypeScript conformance CLI](../ahp-conformance/README.md).
Library users should read
[`deterministic-openai-fixture`](../../crates/deterministic-openai/README.md) next.
