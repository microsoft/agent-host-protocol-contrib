---
name: design-ahp-protocol
description: Design or review Agent Host Protocol commands, actions, state, errors, reducers, schemas, code generation, versioning, compatibility, and multi-client invariants. Use when changing the AHP contract or assessing a protocol-level design, not when implementing only one host, client, or test suite.
license: MIT
---

# Design AHP Protocol

Use this skill for a change to the AHP contract itself: a command, action,
channel state shape, error, reducer rule, schema, generated binding, version
rule, or interoperability invariant.

## Use this / use another skill

| Use this skill | Use another skill |
| --- | --- |
| The requested outcome changes what AHP peers may send, receive, reduce, or negotiate. | Use `build-ahp-host` for an implementation-only server concern. |
| You need to evaluate compatibility, capability gating, or cross-language code generation. | Use `build-ahp-client` for consuming an established contract in an app or SDK client. |
| You are defining the observable contract that tests must enforce. | Use `test-ahp-conformance` to implement black-box probes for an established contract. |

If a task includes design and implementation, settle the contract and its
observable invariants here first, then activate the implementation skill for
the code-owning portion.

## Establish the current authority

1. Identify the protocol version and branch actually targeted; do not infer
   shapes from an old sample, package, or blog post.
2. Locate the protocol source in the workspace. Read its `types/`, canonical
   channel reducers, schemas, `docs/specification/`, `docs/guide/`, version
   registry, and the relevant Rust and TypeScript SDK bindings.
3. If the source is not present, use the released schemas and documentation in
   `microsoft/agent-host-protocol`; state which revision or release informed the
   decision. Do not hard-code a local sibling-repository path.
4. Inspect nearby prior art, existing test cases, generated-file conventions,
   and release/versioning instructions before proposing a new shape.
5. Separate **normative AHP behavior** from profile, host, or sample policy.
   A bounded replay log, a selected persistence engine, or an unsupported
   feature in one host is not automatically a protocol requirement.

## Model the change before editing

Write a compact contract table:

| Question | Decide explicitly |
| --- | --- |
| Scope | Channel URI, command/notification, action, state fields, and ownership |
| Direction | Client request/notification, server response/notification, or both |
| Reducer | Pure state transition, idempotence, replacement/merge/append behavior |
| Ordering | `serverSeq`, origin echo/rejection, and ordering with related actions |
| Failure | JSON-RPC versus rejected action, error code/data, recoverability |
| Compatibility | Version introduction, capability gate, old-peer behavior, omission rules |

Actions are state mutations, not a substitute for vague side effects. Route an
inner action by its envelope's channel; retain action fields that are intrinsic
to the action. Define who assigns identifiers and how multiple clients converge.

For client-dispatchable actions, specify authoritative echo and rejection
semantics. A client may optimistically reduce an action, but it must rebase
pending work on server-authoritative envelopes and undo a rejected prediction.

## State and reducer discipline

- Start from the canonical state and action discriminated unions, not a
  handwritten partial copy.
- Make reducers pure, exhaustive, and immutable. Keep I/O, transport, model
  calls, and persistence outside reducer code.
- Define snapshot and action equivalence: reducing a snapshot followed by
  ordered actions must reach the same observable state for all subscribers.
- State which fields are complete replacements, patches, keyed upserts, or
  append-only histories. Define unknown IDs, duplicate IDs, and terminal-state
  handling.
- Do not let a session catalog summary silently become live session state; use
  the current channel definitions and typed snapshots.

## Versioning and generated artifacts

1. Classify the change as additive, breaking, or clarification under the
   protocol's current SemVer and stability policy.
2. For additive behavior, decide whether a capability is needed before it
   becomes baseline. Older peers must safely ignore behavior they do not know.
3. Ensure negotiation selects one offered version for the whole connection;
   never design per-message renegotiation.
4. Change canonical types, schemas, version registry, and fixtures first.
   Run the repository's generators and commit their outputs only when that
   repository tracks them. Never hand-edit generated bindings as the source of
   truth.
5. Update normative documentation, compatibility notes, and changelog material
   required by the repository's release process.

## Validate the contract

Test the smallest affected layer first: schema/type checks and reducer fixtures.
Then run generated-binding checks and cross-client or wire tests relevant to the
change. Include:

- valid and malformed shapes;
- ordering and repeated delivery;
- old/new compatible peers and unsupported negotiation;
- two clients racing or observing the same resource;
- snapshots versus action replay and rejection reconciliation.

Stop when the types, schema, canonical reducer behavior, generated SDKs, and
normative docs agree. Record intentional scope and non-goals, especially
features left capability-gated or profile-specific.
