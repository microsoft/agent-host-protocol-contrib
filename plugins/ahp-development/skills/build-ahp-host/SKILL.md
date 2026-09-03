---
name: build-ahp-host
description: Implement or review an Agent Host Protocol host or server, including JSON-RPC transport routing, subscriptions, serverSeq and reconnect, durable state, cancellation, concurrency, model adapters, and explicit unsupported behavior. Use for host architecture and server behavior, not for changing the AHP contract or building a client UI.
license: MIT
---

# Build an AHP Host

Use this skill for an AHP server implementation or review: WebSocket/JSON-RPC
entry points, command routing, state publication, persistence, model execution,
and lifecycle behavior.

## Use this / use another skill

| Use this skill | Use another skill |
| --- | --- |
| A host must accept requests, publish actions, resume clients, or run agent work. | Use `design-ahp-protocol` to alter commands, actions, schemas, or compatibility rules. |
| You are deciding persistence, fan-out, cancellation, or adapter boundaries. | Use `build-ahp-client` for application state, UI, and client reconnection code. |
| You need observable end-to-end probes against a host. | Use `test-ahp-conformance` to build the black-box tests. |

## Ground the implementation

1. Determine the negotiated AHP version and supported capabilities from the
   current protocol source. Read the applicable lifecycle, subscription,
   channel, error, and versioning documents plus wire schemas/types.
2. Locate equivalent host code and tests in the current repository before
   inventing a new flow. If the protocol source is absent locally, consult the
   upstream repository and use released schemas; do not depend on a
   machine-specific sibling path.
3. Mark each requirement as normative AHP behavior or this host's profile
   choice. For example, redb, replay retention limits, queue sizes, a narrow
   OpenAI request shape, and excluded feature surfaces are sample choices, not
   universal AHP rules.
4. Map the existing seams: transport, JSON-RPC parser/router, protocol handler,
   domain/turn logic, persistence port, publisher, and model adapter.

## Plan observable invariants

Before changing code, name the externally observable assertions:

| Area | Invariant |
| --- | --- |
| Handshake | Negotiate once; accept an offered compatible version or return the specified error and close. |
| Subscriptions | Return typed snapshots where applicable and send only traffic scoped to active subscriptions. |
| Actions | Assign monotonically increasing `serverSeq`; preserve channel routing and origin/rejection semantics. |
| Durability | Commit state and replayable action atomically before publishing it to peers. |
| Recovery | Replay a complete gap in order, or return replacement snapshots; never send a misleading partial replay. |
| Cancellation | A cancelled turn reaches one terminal state and cannot publish late output. |
| Errors | Invalid requests and unsupported methods fail explicitly; do not return success-shaped no-ops. |

Treat root/session/chat or other channel state as typed protocol projections,
not persistence DTOs. Keep storage records behind a port so schema conversion
and database migration do not leak into routing or turn logic.

## Implement in a safe order

1. Validate JSON-RPC shape, connection phase, negotiated version, channel URI,
   permissions, and action payload before mutating durable state.
2. Route commands by their defined ownership. Preserve request/response
   correlation and distinguish requests from notifications.
3. Serialize the critical publication path sufficiently to keep action order:
   reduce/transition state, durably append the envelope with its sequence, then
   fan it out. Do not hold unrelated network or model work under a global lock.
4. Track subscribers and bounded outbound delivery deliberately. A slow peer
   may be dropped under a host policy, but its reconnection must be able to
   establish a truthful state from replay or snapshots.
5. Implement reconnect from the client's last seen sequence and requested
   subscriptions. A replay gap requires snapshot fallback. Protocol
   notifications are ephemeral; make clients recover their catalog through the
   prescribed fetch/list path rather than pretending notifications replay.
6. Give every running turn a cancellation/shutdown token. Check it before each
   persistence/publication step, remove its registry entry on all exits, and
   suppress deltas, completion, and errors that arrive after cancellation.
7. Isolate model-provider request/response normalization and timeout/error
   mapping behind a small interface. Map provider failures to intentional AHP
   terminal behavior without exposing credentials or provider DTOs.
8. Reject unsupported commands/actions clearly at the appropriate protocol
   boundary. Document the host profile rather than implying an omission is an
   AHP limitation.
9. When models or provider options are selectable, advertise them through root
   model metadata and the session-config commands. Persist resolved defaults,
   validate mutable `session/configChanged` actions, and record the effective
   `ModelSelection` on each user message rather than hiding selection in a
   private endpoint setting.

## Keep the architecture human-scale

Prefer narrow ownership boundaries over a universal dispatcher:

- transport owns peers and connection loops;
- protocol owns parsing, negotiation, and command validation;
- domain/turn logic owns lifecycle decisions;
- persistence owns transactions, replay, and snapshot projections;
- publication owns sequence assignment and fan-out;
- adapters own external model APIs.

Use the repository's canonical types and generated bindings. Change generated
files only through their documented generator.

## Validate and finish

Run focused unit tests for the changed parser, persistence, turn, or publisher
first. Then exercise a raw or SDK client for handshake, snapshot, action
ordering, reconnection, two-client delivery, cancellation, and failure paths.
Run broader host/conformance checks when the focused slice passes.

Stop when no post-cancellation output can appear, every advertised action was
durable first, replay never lies about a gap, and unsupported scope is explicit.
Document operational bounds and non-goals as host-profile choices.
