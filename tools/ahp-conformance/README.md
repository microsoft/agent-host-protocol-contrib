# AHP conformance runner

The runner opens raw WebSocket JSON-RPC connections and checks observable
behavior, so it can probe this sample or another compatible host:

```powershell
cargo run -p ahp-conformance -- --target ws://127.0.0.1:8787/ahp
cargo run -p ahp-conformance -- --local-fixture --format json
```

```bash
cargo run -p ahp-conformance -- --target ws://127.0.0.1:8787/ahp
cargo run -p ahp-conformance -- --local-fixture --format json
```

For `--local-fixture`, the runner starts `deterministic-openai-fixture` and
`advanced-workspace-host` on ephemeral loopback ports with no web root.

## Authenticated targets

Authenticated WebSocket URLs often contain query credentials. Save one without
putting it in shell history:

```powershell
cargo run -p ahp-conformance -- --save-target vscode-local --case negotiation
cargo run -p ahp-conformance -- --target-profile vscode-local
```

```bash
cargo run -p ahp-conformance -- --save-target vscode-local --case negotiation
cargo run -p ahp-conformance -- --target-profile vscode-local
```

The first command prompts for the full `ws://` or `wss://` URL with input
hidden. Profiles are stored in the gitignored
`.ahp-conformance/targets.json`; Unix permissions are set to `0600`. You may
also combine `--target` or `AHP_CONFORMANCE_TARGET` with
`--save-target <name>`. Reports redact URL userinfo and query strings.

If the selected agent advertises OAuth protected resources, provide a bearer
token through the environment. The runner pushes it with AHP's `authenticate`
command before creating each session and never includes it in reports:

```powershell
$env:AHP_CONFORMANCE_AUTH_TOKEN = gh auth token
cargo run -p ahp-conformance -- --target-profile vscode-local
Remove-Item Env:AHP_CONFORMANCE_AUTH_TOKEN
```

```bash
AHP_CONFORMANCE_AUTH_TOKEN="$(gh auth token)" \
  cargo run -p ahp-conformance -- --target-profile vscode-local
```

Plain `ws://` connections use a bounded, validated one-write HTTP upgrade for
compatibility with hosts whose upgrade parser is sensitive to fragmented
handshakes. Secure `wss://` connections use the standard TLS WebSocket client.

The standard suite reports 41 independently named contracts:

- Transport:
  - `ping-before-initialize`
  - `ping-after-initialize`
- Initialization:
  - `initialize-version`
  - `initialize-root-snapshot`
  - `initialize-unsupported-version`
- Authentication:
  - `authentication-invalid-resource`
  - `authentication-advertised-resource` *(not applicable without advertised resources or a supplied token)*
- Sessions and chats:
  - `session-default-chat`
  - `unknown-metadata-ignored`
  - `initial-subscription-snapshots`
  - `root-session-added`
  - `session-duplicate-error`
  - `session-summary-shape`
  - `chat-subscription-snapshot`
  - `session-listing`
  - `session-pagination-limit`
  - `session-pagination-cursor`
  - `session-pagination-invalid-cursor`
  - `session-list-order`
  - `fetch-turns-invalid-cursor`
  - `additional-chat` *(not applicable when `multipleChats` is not advertised)*
  - `root-session-removed`
  - `disposed-session-absent`
  - `unsubscribe-stops-delivery`
- Turn streaming:
  - `turn-started`
  - `response-part-markdown`
  - `response-deltas`
  - `response-part-before-delta`
  - `turn-complete`
  - `root-session-summary-changed`
  - `session-summary-identity-omitted`
- Cancellation:
  - `cancellation-terminal`
  - `cancellation-quiescence`
  - `cancellation-snapshot`
- Synchronization and replay:
  - `multi-client-live-delivery`
  - `reconnect-action-replay`
  - `replay-turn-lifecycle`
  - `subscribe-retains-all-turns`
- JSON-RPC errors:
  - `jsonrpc-parse-error`
  - `jsonrpc-method-not-found`
  - `jsonrpc-invalid-params`

The old compound names remain available as selectors. For example,
`--case negotiation` selects all three initialization contracts, while
`--case initialize-version` selects only that contract. The other group
selectors are `sessions`, `streaming-lifecycle`, `cancellation`,
`synchronization-replay`, and `errors`. The `transport` group selects both ping
contracts.

The local-fixture-only cases are:

- `workspace-validation` *(local only)* — allowed-root URI validation, canonical
  workspace selection, and server-tool schemas
- `replace-string-tool` *(local only)* — exact replacement lifecycle, persisted
  tool result, and a rejected workspace escape
- `shell-approval` *(local only)* — deny and allow-once confirmation behavior
- `shell-approval-persistence` *(local only)* — persisted exact-command approval
  across a sample-host/database restart
- `tool-cancellation` *(local only)* — cancellation of an approved delayed shell
  command before its side effect, including terminal pending-approval state
- `upstream-error` *(local only)* — fixture upstream error propagation
- `model-catalog` *(local only)* — discovered OpenAI models and their
  model-specific `thinkingLevel` schemas
- `session-model-config` *(local only)* — root model-config resolution,
  validation, and filtered completions
- `session-model-default` *(local only)* — resolved session defaults applied to
  model-less turns and persisted turn history
- `session-model-change` *(local only)* — accepted/rejected session changes,
  replay, and active-turn isolation
- `message-model-override` *(local only)* — per-message model selection without
  mutating the session default
- `session-model-restart` *(local only)* — selected defaults retained across an
  advanced-host/redb restart
- `chat-drafts` *(local only)* — model-less session creation, multi-client draft
  sync and replay, causal rejection of stale writes, redb restart persistence,
  clearing, and draft validation
- `draft-send-lifecycle` *(local only)* — accepted turns clear their matching
  draft atomically while rejected turns retain it
- `response-part-ordering` *(local only)* — a v2 fixture tool call is followed
  by a distinct, ordered markdown response part in live and replay state
- `draft-model-only` *(local only)* — an empty-text draft retains its selected
  model and clears by omitting the draft value
- `conversation-history` *(local only)* — a second turn receives prior user and
  assistant messages and can repeat the first assistant response
- `persistence-restart` *(local only)* — session and completed-turn recovery
- `responses-streaming` *(local only)* — true upstream text and function-argument
  streaming with an explicit Responses API host
- `chat-completions-streaming` *(local only)* — the same streaming checks with an
  explicit Chat Completions API host
- `api-auto-selection` *(local only)* — Auto against dual and Chat-only fixtures,
  plus explicit-mode routing behavior
- `streaming-cancellation` *(local only)* — cancellation after the first upstream
  delta under both explicit API modes

The complete `--local-fixture` suite contains 63 independently reported cases:
41 standard contracts and 22 fixture-only cases.

Each result carries a scope label. `protocol-core` covers handshake and
JSON-RPC behavior; `text-chat-profile` covers the session/chat streaming
profile (including `createChat`) exercised by this sample;
`local-fixture-sample-host` covers deterministic upstream and general restart
assertions; `local-fixture-workspace-tools` covers filesystem-sensitive tool
probes; `local-fixture-model-streaming` covers explicit API routing and true
upstream streaming; and `local-fixture-model-configuration` covers discovered
models plus root, session, and per-message model configuration; and
`local-fixture-chat-drafts` covers durable draft state, conversation history,
and tool-to-text response-part ordering. All cases marked local only require
`--local-fixture`. An optional capability that is not advertised is reported as
`not_applicable` rather than passed, failed, or skipped.

### Protocol doc-comment coverage

The standard cases are derived from the observable AHP 0.9 contracts documented
by `ahp-types`. This pass covers negotiation, unknown metadata, protected-resource
authentication, initial and explicit subscriptions, session catalogue shape and
ordering, pagination, session lifecycle notifications, duplicate creation,
history cursors, action unsubscription, response-part ordering, summary-change
notifications, retained-turn snapshots, replay, cancellation, and JSON-RPC
errors.

Requirements that only apply when a host advertises another protocol surface
are intentionally not treated as passing when that surface is absent. Dedicated
capability-gated scenarios are still needed for resource operations and watches,
OTLP telemetry, MCP Apps, terminals, changesets, annotations, automations, and
tunnels. Those scenarios need capability predicates and, for filesystem or
process mutations, explicit side-effect and cleanup controls before they are
safe to run against arbitrary hosts.

By default model text is not asserted, which makes the standard suite useful
against a well-behaving real host/model. `--model-checks` enables exact output
comparison for a target that intentionally implements the documented
`[AHP-TEST v1]` instruction block. `--case` may be repeated to select cases;
an unknown case or a local-only case without `--local-fixture` is an error.
`--timeout-ms` bounds every connect/read.

Deterministic exact assertions (including mock SSE chunk count) are separate
from real-model semantic assertions. The host may choose different
`chat/delta` boundaries than fixture SSE chunk boundaries.

`api-auto-selection` uses the dual fixture and a Chat-only fixture. The
fixture does not publicly expose endpoint counters, so the runner proves
end-to-end Auto success and Chat-only fallback while adapter unit tests cover
the Responses preference. Auto falls back only for Responses HTTP `404`, `405`,
or `501`; explicit `Responses` and `ChatCompletions` modes never fall back.

The model-configuration probes use the dual fixture's fixed `/v1/models`
response. The local stack does not inject catalog HTTP status failures; 404,
405, and 501 fallback remains covered by the advanced-host unit tests.

The v2 fixture produces a tool round followed by its final-text round; it
cannot model text before a tool in the same round. `response-part-ordering`
therefore covers the reported tool-to-text ordering regression.

Internally, `runner/suite.rs` selects and reports cases, `basic.rs`,
`session_contracts.rs`, `turns.rs`, `advanced.rs`, `model_config/`, and
`tools/` own cohesive probe groups, and `probe.rs` owns WebSocket request
correlation and bounded frame reads. The local stack only starts/stops the
fixture/sample host and manages temporary files; every host assertion uses raw
WebSocket JSON-RPC frames and never inspects host storage.

**Audience:** host implementers verifying observable compatibility. Start with
`src/main.rs`, then `src/runner.rs`, and use `--case` to isolate one scope.
Read the [architecture](../../docs/architecture.md) for behavior being tested
and the [advanced host guide](../../samples/rust/advanced-workspace-host/README.md)
for the local-fixture composition.
