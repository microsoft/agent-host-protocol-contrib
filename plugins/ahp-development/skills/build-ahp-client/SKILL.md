---
name: build-ahp-client
description: Implement or review an SDK-backed Agent Host Protocol client application, including initialization, subscriptions, immutable reducer state, reconnect cleanup, multi-host behavior, and UI-friendly projections. Use for browser, TypeScript, desktop, or other client consumers, not for host internals or protocol contract changes.
license: MIT
---

# Build an AHP Client

Use this skill when consuming AHP from an application or client integration.
It applies naturally to TypeScript and browser clients, but the state and
transport rules are cross-language.

## Use this / use another skill

| Use this skill | Use another skill |
| --- | --- |
| The client must connect, initialize, subscribe, render, dispatch, or reconnect. | Use `build-ahp-host` for server routing, persistence, fan-out, or model execution. |
| You need a new action, state field, wire command, or compatibility rule. | Use `design-ahp-protocol` before changing the contract. |
| You are authoring raw WebSocket black-box probes or fixture-driven conformance coverage. | Use `test-ahp-conformance`. |

## Ground in the active protocol

1. Identify the current AHP version, SDK release, and target host capabilities.
   Read the protocol's lifecycle, subscriptions, reconciliation, state-model,
   and versioning documentation plus the generated client APIs.
2. Find existing connection, store, reducer, and projection code in the
   repository. Reuse the SDK's client, transport, state mirror, and canonical
   reducers when they fit instead of duplicating wire handling.
3. If protocol source is not in the workspace, consult the upstream
   `microsoft/agent-host-protocol` documentation and schemas. Never assume a
   particular local checkout exists.
4. Separate AHP guarantees from local product policy: reconnect backoff, UI
   limits, retention assumptions, and a particular host's unsupported surfaces
   are not portable requirements.

## Design the client state boundary

Maintain a protocol layer and a presentation layer:

| Layer | Responsibility |
| --- | --- |
| Transport/client | Connection lifecycle, JSON-RPC requests, subscriptions, and shutdown |
| Protocol store | Typed snapshots, ordered envelopes, canonical reducers, sequence tracking |
| Projection | Fresh immutable view models, sort/filter/format policy, UI selection state |
| UI | Renders projections and invokes application commands; never mutates protocol maps |

Keep snapshots typed by channel/state rather than treating all JSON as one
untyped store. Treat the server's action envelope as authoritative and route it
using its channel URI. Do not invent a channel from fields on the inner action.

## Connection and subscription flow

1. Create one durable client identity per logical client according to the
   protocol, then open the selected host transport.
2. Initialize with the SDK's supported protocol-version list, root channel,
   and useful initial subscriptions. Record the selected version and each
   snapshot's `fromSeq`/server sequence.
3. Attach local subscription consumers before or as the SDK requires so no
   actions are lost between handshake and consumption. Close them deliberately
   when the user leaves a resource or disconnects.
4. Subscribe on demand to session, chat, or other resources. Apply a returned
   snapshot before relying on later actions, and retain the desired
   subscription set for reconnect.
5. Fetch or list catalog data where the protocol requires it. Do not depend on
   ephemeral notifications being replayed.

## Reduce and reconcile correctly

- Apply only envelopes newer than the last observed `serverSeq`; update the
  sequence monotonically.
- Use immutable reducers and immutable/frozen UI projections. Mutable SDK
  mirrors or maps are protocol internals, not React or UI snapshots.
- If using optimistic dispatch, keep confirmed state and ordered pending
  actions separately. Match your own authoritative echo, rebase remaining
  actions over foreign/server changes, and remove/revert a rejected action.
- Respect terminal turn states. A cancel affordance should not make a late
  delta or completion appear valid after the host has terminated the turn.
- Keep multi-host data isolated by connection/host identity. Reset or
  namespace state, sequence numbers, subscriptions, and selected resources
  when switching hosts.

## Reconnect without stale events

Use a connection epoch/token (or equivalent ownership guard):

1. Invalidate the old epoch before closing a socket or starting another.
2. Close async iterators and subscriptions from the old client.
3. Ignore delayed callbacks, errors, and action streams unless they still
   belong to the current epoch and current client.
4. Reconnect with the durable client ID, last observed sequence, and desired
   subscriptions.
5. If the host returns replay, reduce every envelope in order; if it returns
   snapshots because the replay gap is unavailable, replace confirmed state and
   clear/reconcile pending work as the protocol requires. Remove resources
   named missing by the host.
6. Re-fetch catalog state after reconnect because protocol notifications are
   not replayed.

For configurable agents, render the host's `SessionModelInfo.configSchema` and
session configuration schema instead of hardcoding provider-specific values.
Use `resolveSessionConfig` and `sessionConfigCompletions`, preserve unknown
configuration entries when editing known ones, and attach the effective
`ModelSelection` to the user message so history records what actually ran.

## Validate and document

Test the smallest layer first: reducer/projection purity, sequence rejection,
and epoch cleanup. Then test initialization, delayed subscription events,
reconnect replay and snapshot fallback, two-client updates, host switching, and
cancel/error rendering. Run the application's broader typecheck/build/tests
after focused tests pass.

Document unsupported host capabilities, degraded UI behavior, and intentional
local policy. Stop when an old socket cannot update the new connection and the
UI always sees a fresh immutable projection of authoritative protocol state.
