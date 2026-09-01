# AHP conformance runner

The runner opens raw WebSocket JSON-RPC connections and checks observable
behavior, so it can probe this sample or another compatible host:

```powershell
cargo run -p ahp-conformance -- --target ws://127.0.0.1:8787/ahp
cargo run -p ahp-conformance -- --local-fixture --format json
```

For `--local-fixture`, the runner starts `deterministic-openai-fixture` and
`advanced-workspace-host` on ephemeral loopback ports with no web root.

The standard cases are:

- `negotiation` — compatible and incompatible initialize offers
- `sessions` — root/session/chat snapshots plus session listing
- `streaming-lifecycle` — response part, deltas, and completion ordering
- `cancellation` — cancellation while a delayed completion is pending, followed
  by a bounded post-cancel quiescence check
- `synchronization-replay` — live second-client delivery and reconnect replay
- `errors` — malformed JSON, unsupported method, and invalid parameters
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
- `persistence-restart` *(local only)* — session and completed-turn recovery
- `responses-streaming` *(local only)* — true upstream text and function-argument
  streaming with an explicit Responses API host
- `chat-completions-streaming` *(local only)* — the same streaming checks with an
  explicit Chat Completions API host
- `api-auto-selection` *(local only)* — Auto against dual and Chat-only fixtures,
  plus explicit-mode routing behavior
- `streaming-cancellation` *(local only)* — cancellation after the first upstream
  delta under both explicit API modes

Each result carries a scope label. `protocol-core` covers handshake and
JSON-RPC behavior; `text-chat-profile` covers the session/chat streaming
profile (including `createChat`) exercised by this sample;
`local-fixture-sample-host` covers deterministic upstream and general restart
assertions; `local-fixture-workspace-tools` covers filesystem-sensitive tool
probes; `local-fixture-model-streaming` covers explicit API routing and true
upstream streaming; and `local-fixture-model-configuration` covers discovered
models plus root, session, and per-message model configuration. All cases
marked local only require `--local-fixture`.

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

Internally, `runner/suite.rs` selects and reports cases, `basic.rs`, `turns.rs`,
`advanced.rs`, `model_config/`, and `tools/` own cohesive probe groups, and
`probe.rs` owns WebSocket request correlation and bounded frame reads. The
local stack only starts/stops the fixture/sample host and manages temporary
files; every host assertion uses raw WebSocket JSON-RPC frames and never
inspects host storage.

**Audience:** host implementers verifying observable compatibility. Start with
`src/main.rs`, then `src/runner.rs`, and use `--case` to isolate one scope.
Read the [architecture](../../docs/architecture.md) for behavior being tested
and the [advanced host guide](../../samples/rust/advanced-workspace-host/README.md)
for the local-fixture composition.
