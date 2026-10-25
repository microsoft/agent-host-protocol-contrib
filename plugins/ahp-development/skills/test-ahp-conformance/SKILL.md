---
name: test-ahp-conformance
description: Create or review black-box Agent Host Protocol conformance tests, including deterministic model fixtures, lifecycle and error cases, multi-client and restart coverage, bounded timeouts, and CLI/HTML reports with optional JSON results. Use for observable interoperability testing, not for changing protocol design or implementing host/client production code.
license: MIT
---

# Test AHP Conformance

Use this skill to verify what an AHP peer does on the wire. Prefer raw
WebSocket/JSON-RPC probes or an independently useful test harness over tests
that inspect host internals.

## Use this / use another skill

| Use this skill | Use another skill |
| --- | --- |
| The outcome is a black-box assertion, fixture contract, report, or interoperability suite. | Use `design-ahp-protocol` to define or alter normative behavior. |
| You are fixing server implementation code, durability, or cancellation. | Use `build-ahp-host`. |
| You are changing app state, SDK consumption, or UI reconnection behavior. | Use `build-ahp-client`. |

If a test exposes an ambiguity, capture the observed evidence and defer the
normative decision to `design-ahp-protocol`; do not silently make a sample
behavior universal.

## Ground the test oracle

1. Target the released specification matching the installed SDK, currently
   AHP **1.0.0** (`@microsoft/agent-host-protocol` 1.0.0). Identify the negotiated
   capabilities and relevant channel schemas. Read the matching release source
   and normative documents first, not protocol HEAD. A protocol checkout used
   for authoring tests MUST be at the matching release tag/commit (for 1.0.0,
   `5f16d81b`, `release: AHP 1.0.0`). Use the same release when consulting
   upstream `microsoft/agent-host-protocol` schemas/docs outside the workspace;
   verify spec anchors and MUST/SHOULD classification against that version.
   Behavior that is only on protocol `main` is **proposed**: put it in
   `src/tests/proposed/`, gate it with `requires.proposed` (run with `--proposed`),
   connect with `proposedClient(t)`, and skip when the host doesn't implement it.
2. Inspect the TypeScript runner's `TestDefinition`s in
   `tools/ahp-conformance/src/tests/<area>/`, reports, and fixture contracts.
   Keep the runner independent of host storage, private functions, and
   implementation-specific timing. See the
   [runner README](../../../../tools/ahp-conformance/README.md) for configuration
   and usage.
3. Classify each assertion:

| Scope | Assert |
| --- | --- |
| Protocol core | JSON-RPC validity, negotiation, channel/subscription and ordering semantics |
| Feature/profile | A documented channel or host profile such as text chat |
| Local fixture/sample | Deterministic upstream output, persistence setup, or deliberate host limits |

Never label a sample's bounded replay buffer, model API subset, or excluded
feature as an AHP MUST unless the matching released normative source says so.

## Define observable invariants

Write each case as precondition → wire inputs → bounded observations →
pass/fail detail. Favor stable relationships over implementation timing:

- selected version is one offered by the client; incompatible offers produce
  the defined error and connection outcome;
- a subscription receives a typed snapshot and actions scoped to its channel;
- action envelopes have monotonically increasing `serverSeq`, correct origin,
  and valid lifecycle ordering;
- a second subscribed client observes authoritative actions;
- reconnect returns a complete ordered replay or replacement snapshots, never a
  partial gap presented as replay;
- a cancelled turn has its terminal cancellation behavior and emits no later
  delta, completion, or error;
- malformed JSON, invalid parameters, and unsupported methods produce explicit
  JSON-RPC failures rather than success-shaped no-ops.

Use exact field/value assertions for contractually fixed data and semantic
assertions for host-selected IDs, timestamps, chunk boundaries, or model text.

## Use deterministic models carefully

A deterministic fixture is a test dependency with a versioned syntax, not a
claim about ordinary model behavior. Keep its directive grammar explicit and
immutable. The `[AHP-TEST v1]` fixture convention can request exact output,
chunk count, delay, or an upstream error; only tests intentionally targeting
that fixture should assert those exact results. Model-backed suite tests use
natural-language prompts from `tools/ahp-conformance/src/harness/prompts.ts`
with embedded `[AHP-TEST vN]` directives, so the same probes work against real
models and the fixture. The fixture's v4 directives cover recall, read-file,
and write-file behavior.

For a real model or a generally compatible host:

- assert lifecycle and terminal semantics, not exact prose;
- do not require AHP `chat/delta` boundaries to equal upstream SSE boundaries;
- make model-specific exact checks opt-in and label them clearly;
- avoid tests that reproduce the host's private parser, reducer, or database
  logic, because they become tautological.

## Build reliable probes

1. Add one behavior per `TestDefinition` under
   `tools/ahp-conformance/src/tests/<area>/`. Group tests into Transport &
   lifecycle, Root & sessions, Chats & turns, Resources, Tools & permissions,
   or Authentication. Gate capability-dependent tests with `requires.*` so
   unsupported capabilities are skipped.
2. Use request IDs to correlate responses and collect notifications through a
   single bounded frame reader.
3. Give every connect, request, read, wait, process restart, and teardown a
   timeout with an actionable case name in failures.
4. Isolate state using unique client/resource/turn identifiers. Run
   multi-client cases with independent transports and state trackers.
5. Make ordering assertions from `serverSeq` and action lifecycle, not sleeps.
   Use a controlled fixture delay only to create a meaningful cancellation or
   concurrent-observer window.
6. Preserve CLI/HTML reporting with optional JSON results, test identity,
   status, and concise evidence. The live HTML report shows queued, running,
   passed, failed, and skipped tests with logs and wire transcripts. Mark
   unavailable profile/local-only cases as skipped rather than passing them
   accidentally.

## Validate in layers

Use Node 24+; no build step is needed:

```bash
cd tools/ahp-conformance
npm install
node src/cli.ts --init
node src/cli.ts ahp-conformance.yml --list
node src/cli.ts ahp-conformance.yml --filter '<glob>'
```

The YAML config selects the WebSocket URL, headers/query, optional
`host.command`, AHP `authenticate` tokens from prompt/env/value/command,
agent/model, scratch workspace (OS tmpdir by default), concurrency, timeouts,
filters, and reporter. The default path is `./ahp-conformance.yml`; if it is
missing, configuration is offered interactively. `npm start -- …` is also
supported.

Run the smallest selected test while authoring it, then its cohesive group.
Run the full external-host-safe suite without model-specific exact assertions.
Use `node src/cli.ts examples/advanced-workspace-host.yml` for the sample stack;
its config launches `advanced-workspace-host --demo` with the embedded fixture.
Use deterministic upstream-error, restart, and cancellation checks where the
host supports them. Confirm cleanup leaves no listeners, processes, or
persistent test state behind.

Interactive runs default to a live browser HTML report; otherwise the CLI is
used. Select `--reporter cli|html`, suppress browser opening with `--no-open`,
and optionally write results with `--json <file>`. Failures exit with code 1.

Stop when each failure maps to a visible wire observation and report. Document
the suite's target protocol range, fixture-only cases, profile assumptions, and
intentional non-goals.
