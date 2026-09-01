---
name: test-ahp-conformance
description: Create or review black-box Agent Host Protocol conformance tests, including deterministic model fixtures, lifecycle and error cases, multi-client and restart coverage, bounded timeouts, and text or JSON reports. Use for observable interoperability testing, not for changing protocol design or implementing host/client production code.
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

1. Identify the target protocol version, negotiated capabilities, and relevant
   channel schemas. Read the current protocol source and normative documents
   first; use upstream `microsoft/agent-host-protocol` schemas/docs if source is
   not in the workspace.
2. Inspect existing runner probes, test cases, reports, and fixture contracts.
   Keep the runner independent of host storage, private functions, and
   implementation-specific timing.
3. Classify each assertion:

| Scope | Assert |
| --- | --- |
| Protocol core | JSON-RPC validity, negotiation, channel/subscription and ordering semantics |
| Feature/profile | A documented channel or host profile such as text chat |
| Local fixture/sample | Deterministic upstream output, persistence setup, or deliberate host limits |

Never label a sample's bounded replay buffer, model API subset, or excluded
feature as an AHP MUST unless the current normative source says so.

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
that fixture should assert those exact results.

For a real model or a generally compatible host:

- assert lifecycle and terminal semantics, not exact prose;
- do not require AHP `chat/delta` boundaries to equal upstream SSE boundaries;
- make model-specific exact checks opt-in and label them clearly;
- avoid tests that reproduce the host's private parser, reducer, or database
  logic, because they become tautological.

## Build reliable probes

1. Use request IDs to correlate responses and collect notifications through a
   single bounded frame reader.
2. Give every connect, request, read, wait, process restart, and teardown a
   timeout with an actionable case name in failures.
3. Isolate state using unique client/resource/turn identifiers. Run
   multi-client cases with independent transports and state trackers.
4. Make ordering assertions from `serverSeq` and action lifecycle, not sleeps.
   Use a controlled fixture delay only to create a meaningful cancellation or
   concurrent-observer window.
5. Emit text and machine-readable reports with case name, scope, target,
   selected options, status, and concise evidence. Mark unavailable
   profile/local-only cases as skipped rather than passing them accidentally.

## Validate in layers

Run the smallest selected case while authoring it, then its cohesive group.
Run the full external-host-safe suite without model-specific exact assertions.
When available, run a local fixture stack for deterministic upstream-error,
restart, and cancellation checks. Confirm cleanup leaves no listeners,
processes, or persistent test state behind.

Stop when each failure maps to a visible wire observation and report. Document
the suite's target protocol range, fixture-only cases, profile assumptions, and
intentional non-goals.
