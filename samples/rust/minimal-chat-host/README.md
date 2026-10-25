# Minimal AHP Responses chat host

> **Educational sample — not production hardened.** Read this when learning the
> smallest useful Agent Host Protocol (AHP) chat loop, not as a server template.

This sample exposes `ws://127.0.0.1:8787/ahp` and exactly one process-local
session and chat:

```text
ahp-session:/minimal-chat-host
ahp-chat:/minimal-chat-host
```

It sends the retained chat turns plus the new user message to the **OpenAI
Responses API** at `POST /v1/responses`; there is no Chat Completions mode or
fallback.

**Audience:** developers learning their first AHP text-streaming host.

## Run it

With no `--openai-base-url`, the host uses `https://api.openai.com` and requires
an API key. With no `--openai-model`, it uses `gpt-5.6-luna`. The host appends
`/v1/responses` to the API origin.

```powershell
$env:OPENAI_API_KEY = "<key>"
cargo run -p minimal-chat-host
```

```bash
OPENAI_API_KEY="<key>" cargo run -p minimal-chat-host
```

Pass `--openai-base-url` and `--openai-model` to override those defaults. An
explicit OpenAI-compatible base URL may omit the API key.

For a deterministic local upstream, start the repository fixture in one
terminal, then this host in another. The sample has no dependency on the
fixture or its support crate.

```powershell
cargo run --manifest-path tools/deterministic-openai/Cargo.toml

cargo run --manifest-path samples/rust/minimal-chat-host/Cargo.toml -- `
  --openai-base-url http://127.0.0.1:8899 `
  --openai-model fixture-model
```

```bash
cargo run --manifest-path tools/deterministic-openai/Cargo.toml

cargo run --manifest-path samples/rust/minimal-chat-host/Cargo.toml -- \
  --openai-base-url http://127.0.0.1:8899 \
  --openai-model fixture-model
```

The fixture replies to ordinary text with a deterministic echo. Connect an AHP
WebSocket client, initialize and subscribe to the chat, then send this
notification:

```json
{
  "jsonrpc": "2.0",
  "method": "dispatchAction",
  "params": {
    "channel": "ahp-chat:/minimal-chat-host",
    "clientSeq": 1,
    "action": {
      "type": "chat/turnStarted",
      "turnId": "example-turn",
      "startedAt": "2026-08-31T00:00:00.000Z",
      "message": { "text": "Hello", "origin": { "kind": "user" } }
    }
  }
}
```

## Supported AHP subset

All commands below are JSON-RPC requests targeting the stated channel.

| Method | Availability | Behavior |
| --- | --- | --- |
| `ping` | before or after initialization | Accepts `ahp-root://`; replies with `null`. |
| `initialize` | once per connection | Uses `ahp-types` 1.0 to select the highest offered version compatible with the 1.0.0 or 0.9.0 baseline and may include root, session, and chat snapshots from `initialSubscriptions`. |
| `subscribe` | after initialization | Accepts only the root, fixed session, or fixed chat URI and returns that typed snapshot. |
| `listSessions` | after initialization | On `ahp-root://`, returns the one fixed session summary. |
| `dispatchAction` | notification after initialization | Accepts only `chat/turnStarted` and `chat/turnCancelled` on the fixed chat. |

`chat/turnStarted` requires a non-empty `turnId`, `startedAt`, and a user
message. The host publishes:

1. `chat/turnStarted` (with the client's `origin`);
2. an empty markdown `chat/responsePart`;
3. one `chat/delta` for each received `response.output_text.delta`;
4. `chat/turnComplete`, or `chat/error` for HTTP, transport, malformed SSE,
   `response.failed`, `response.incomplete`, or `error` events.

Cancelling the active `turnId` emits `chat/turnCancelled`, aborts and drops the
task that owns the reqwest response stream, and prevents later deltas from
being published. `serverSeq` is monotonic for this process. The server fans
out each action to currently subscribed peers only.

On the fixed chat, malformed, unsupported, or inapplicable client actions are
echoed as an `action` envelope with the original `origin` and
`rejectionReason`, so an optimistic client can remove its local action. Actions
for other chat URIs remain ignored. Snapshot creation, subscription, and its
JSON-RPC reply are queued under the same publication boundary; a subscriber
therefore sees its snapshot reply before any newly subscribed action, and every
later action has `serverSeq > fromSeq`.

The SSE decoder accepts fragmented HTTP chunks, CRLF or LF lines, comments,
and multiline `data` fields. It intentionally parses only
`response.output_text.delta`, `response.completed`, `response.failed`,
`response.incomplete`, and `error`; other Responses events are ignored.

## One text turn

```text
AHP client             protocol.rs + host.rs              responses.rs / OpenAI
    | dispatchAction turnStarted |                                  |
    |--------------------------->|                                  |
    |  action: turnStarted       |                                  |
    |<---------------------------| POST /v1/responses stream:true   |
    |  action: responsePart      |--------------------------------->|
    |<---------------------------|                                  |
    |  action: delta             | response.output_text.delta       |
    |<---------------------------|<---------------------------------|
    |  action: turnComplete      | response.completed               |
    |<---------------------------|<---------------------------------|
```

## Intentional omissions

There is no reconnect or action replay (`reconnect` and every unsupported
request receive JSON-RPC `MethodNotFound`), session/chat creation or disposal,
unsubscription, tools, workspaces, shell access, UI/static files, or persistent
storage. With the implied production OpenAI endpoint, successful turns retain
an in-memory `previous_response_id`, so follow-up requests send only the new
user text. Explicit compatible endpoints instead receive user and assistant
text from at most eight finished turns followed by the new user text. Either
form of history exists only until the process exits.

Only one text turn can be active globally. It is deliberately a simple
best-effort multi-client fanout, with no backpressure policy, authentication,
authorization, origin policy, rate limits, retry logic, observability, or
production-grade SSE adapter. Production hosts should put a reusable,
fully-specified Responses adapter and stronger lifecycle/recovery controls
around this shape.

## Read next and checks

1. `src/main.rs` parses the three upstream settings and starts the host.
2. `src/host.rs` composes the fixed process-local state, WebSocket runtime, and
   action fanout.
3. `src/protocol.rs` routes JSON-RPC, makes the three snapshots, and starts or
   cancels the two supported chat actions.
4. `src/responses.rs` constructs the Responses request, decodes SSE, and
   forwards text or terminal results.
5. `tests/end_to_end.rs` provides a tiny in-test Responses SSE server and proves
   stream ordering, initialization/subscription boundaries, rejection echoes,
   and cancellation behavior.

Next, read the repository [reading path](../../../docs/reading-path.md), then
[`ahp-server-core`](../../../crates/ahp-server-core/README.md) to add durable
subscriptions and reconnect/replay. Do not add tools or static hosting to this
sample; those are taught by the advanced composition sample.

```powershell
cargo fmt --manifest-path samples/rust/minimal-chat-host/Cargo.toml -- --check
cargo clippy --manifest-path samples/rust/minimal-chat-host/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path samples/rust/minimal-chat-host/Cargo.toml
```

```bash
cargo fmt --manifest-path samples/rust/minimal-chat-host/Cargo.toml -- --check
cargo clippy --manifest-path samples/rust/minimal-chat-host/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path samples/rust/minimal-chat-host/Cargo.toml
```
