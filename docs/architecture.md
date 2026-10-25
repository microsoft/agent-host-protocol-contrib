# Architecture

This document describes the persistent, advanced path. Begin with
[`minimal-chat-host`](../samples/rust/minimal-chat-host/README.md) if the
protocol lifecycle is new; that sample deliberately does not use this full
stack.

## Pieces and direction

`advanced-workspace-host` is the place where the pieces meet:

```text
ahp-chat browser --WebSocket--> ahp-server-ws --> ahp-server-core
                                                |        |
                                                |        +--> openai-agent-model
                                                |        +--> workspace-agent-tools
                                                +--> ahp-server-redb

advanced-workspace-host also serves the built ahp-chat files.
```

The core owns AHP commands, session/chat lifecycle, subscriptions, action
ordering, reconnect rules, and turn orchestration. It asks a store to save
state, a model adapter for model events, and a workspace profile for tool work.
The WebSocket crate only moves JSON-RPC text and applies the Origin policy.
The redb crate only owns durable records, journal, recovery, and approvals.
The model and workspace crates are independent helpers. This direction keeps
the core usable with another transport or durable store.

Starting a turn atomically returns the completed turns that preceded it. The
core projects their user messages and ordered markdown response parts into the
portable OpenAI request before appending the current workspace-qualified user
message. Production OpenAI Responses mode also persists the last successful
response ID per chat and normally sends only the new input with
`previous_response_id`; the complete message list remains available for Chat
Completions fallback. Because redb supplies both forms after restart, model
context follows the durable AHP chat rather than the lifetime of a host process.

Static files are intentionally absent from the reusable crates: serving the
`ahp-chat` build is an advanced-host composition choice, not an AHP
requirement. `/ahp` is registered before the SPA fallback. A missing
`dist/index.html` produces setup guidance at `/` while the WebSocket host
continues to run.

The `advanced-workspace-host --demo` command adds one more CLI-only
composition: it starts `deterministic-openai-fixture` on loopback and, when
needed, invokes npm to build `ahp-chat`. Neither `AdvancedHost` nor any
reusable crate starts Node, npm, or the fixture; library consumers supply
their own model configuration and web root.

## Minimal streamed text turn

The minimal sample has a fixed session/chat and sends only new user text to
Responses. Its action sequence is:

```text
AHP client              minimal host                    Responses endpoint
    | dispatchAction turnStarted |                              |
    |--------------------------->|                              |
    | <--- chat/turnStarted -----| POST /v1/responses, stream   |
    | <--- chat/responsePart ----|----------------------------->|
    | <--- chat/delta -----------| <--- response.output_text.delta
    | <--- chat/turnComplete ----| <--- response.completed      |
```

The minimal host has no replay or tools; cancellation drops its request task
and later deltas are not published. Its full supported subset is documented
beside the sample.

## Advanced atomic transition and replay

For every state-changing advanced operation, the core receives committed
envelopes before any peer can observe them:

```text
model event
    -> ahp-server-core handles lifecycle change
    -> redb transaction: state + contiguous journal envelopes + serverSeq commit
    -> core receives committed envelopes
    -> live subscriber fanout
```

If the process stops before the commit, neither the snapshot change nor its
actions exist. If fanout fails after a commit, a reconnect can replay the
journal or request a replacement snapshot. `RedbHostStore` retains 2,048
envelopes; an older cursor receives `SnapshotRequired`. Startup recovers an
active interrupted turn as terminal actions in a new atomic transition.

Snapshots expose a pending tool confirmation as `InputNeeded`. A session
summary also promotes a blocked or error child chat, even if its newest or
default chat is idle.

Subscriptions have bounded outbound queues. The core is transport-independent;
`ahp-server-ws` creates one logical peer per WebSocket and releases it when
the connection closes or fails. Browser Origins matching `Host` are accepted
by default, configured HTTP(S) Origins are additive, and native clients with
no `Origin` header are accepted.

For deterministic shutdown—particularly so Windows releases the database
file—`RunningAdvancedHost::shutdown` first cancels and awaits Axum, then calls
`CoreHost::shutdown_and_wait` to join outstanding turn, model, tool, and
approval tasks, and finally closes redb. Dropping the running host only begins
best-effort cancellation.

## Model APIs and timeouts

`openai-agent-model` supports Responses input items and Chat Completions
messages, with SSE text and function-argument events. `auto` starts with
Responses and falls back to Chat Completions **only** when the *initial*
Responses request returns HTTP 404, 405, or 501. Explicit `responses` and
`chat-completions` modes never fall back; authentication, validation,
rate-limit, network, server, malformed-stream, and mid-stream failures never
select the other API.

The connect timeout covers establishing the connection and receiving initial
headers. After headers, an idle timeout is reset by every non-empty body chunk,
including keepalive bytes. There is no total request or stream deadline, so a
healthy stream may run indefinitely.

## Approved shell tool call

Workspace tools are an advanced feature. A shell command is approved using its
exact, trimmed command string, never merely its executable name:

```text
model function call
    -> core records chat/toolCallStart and argument deltas
    -> workspace tools preflight command in selected workspace
    -> core asks client for confirmation
    -> client allows once / always exact command / denies
    -> redb commits decision and tool lifecycle action
    -> workspace tools run command, then core commits result and fans it out
```

An always-allow decision is persisted host-wide in redb for that exact string;
`git status` does not approve `git` or `git status --short`. `replace_string`
is confined to the selected canonical workspace and needs exactly one match.
`run_shell` starts in that workspace, but it is **not an OS sandbox**: the
host process's permissions still apply. It has no stdin, has bounded output,
and cancellation/timeout terminates its process tree.

## Deterministic checks

The deterministic fixture owns versioned directive contracts, including
`[AHP-TEST v1]` text/errors, `[AHP-TEST v2]` sequential tool calls, and
`[AHP-TEST v4]` recall/read-file/write-file behavior. Exact text and chunk
expectations belong to that fixture only. The YAML-configured TypeScript
conformance runner uses granular raw WebSocket JSON-RPC probes and skips tests
for capabilities the host does not advertise. Model-backed probes combine
natural-language prompts with embedded `[AHP-TEST vN]` directives so they work
with real models and the fixture; real-model assertions remain semantic. See the
[fixture](../tools/deterministic-openai/README.md) and
[conformance runner](../tools/ahp-conformance/README.md) for contracts and usage.
