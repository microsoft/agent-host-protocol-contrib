# ahp-server-core

Transport-independent AHP 0.8 host lifecycle logic. A transport creates a logical
peer, passes JSON-RPC text to `CoreHost::handle_text`, and forwards strings from
the peer's bounded outbound receiver.

`HostStore` is the persistence port. Its transition methods atomically update
state and append ordered `ActionEnvelope`s; core only fans out returned,
committed envelopes. `MemoryHostStore` is a complete in-memory implementation
for embedding and tests; production hosts can implement the same narrow port
with redb or another durable store.

`AgentProfile` is the environment port. `WorkspaceAgentProfile` adapts
`workspace-agent-tools` for workspace selection, preflight, exact-command
approval identity, and cancellation-aware execution (including process-tree
termination supplied by that crate).

**Audience:** host authors adding AHP lifecycle, subscriptions, replay, and
turn orchestration without choosing an HTTP or WebSocket stack. Start at
`src/lib.rs`, then read `host.rs`, `turns.rs`, and `store.rs`. Next, read
[`ahp-server-ws`](../ahp-server-ws/README.md) for transport and
[`ahp-server-redb`](../ahp-server-redb/README.md) for durable transitions; the
repository [architecture](../../docs/architecture.md) shows their ordering.
