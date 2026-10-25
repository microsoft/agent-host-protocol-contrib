# ahp-server-core

Transport-independent AHP 1.0 host lifecycle logic. A transport creates a logical
peer, passes JSON-RPC text to `CoreHost::handle_text`, and forwards strings from
the peer's bounded outbound receiver.

`HostStore` is the persistence port. Its transition methods atomically update
state and append ordered `ActionEnvelope`s; core only fans out returned,
committed envelopes. `MemoryHostStore` is a complete in-memory implementation
for embedding and tests; production hosts can implement the same narrow port
with redb or another durable store.

Session title, read, and archived actions are persisted independently, echoed
with their origin, and reflected in session snapshots, root catalog summaries,
and `root/sessionSummaryChanged`. Active clients can join, refresh, or leave;
`createSession.activeClient` eagerly registers the initialized client, and
unsubscribe/disconnect or backpressure eviction removes its membership.
Chat read flags are independent of sibling chats and the session; starting a
turn clears only that chat's read bit. Full session chat summaries, including
working directories, match the chat snapshot. Root summaries carry the complete
ordered compact chat catalog, exact status bits, and default chat. Session
disposal journals `session/chatRemoved` for every cataloged chat before root
removal.

Pending tool confirmations and client-owned tool execution are mirrored into
`session/inputNeededSet` / `session/inputNeededRemoved` atomically with chat
state. Entries carry the complete tool call, turn, and chat; an initialized
session-only subscriber may confirm on the named chat without subscribing to it.
Confirmations require a subscription to either that chat or its owning session;
unsubscribed peers cannot resolve requests or persist command approvals.
Input queue ordering and snapshots follow the AHP 1.0 upsert/removal reducers.

Activity descriptions are not published: snapshots and actions consistently
omit this optional field.
`listSessions` omits `nextCursor` on its final page. Incompatible protocol
negotiation queues the required error before closing the peer.

Chat history supports `chat/truncated` (including clearing all turns); the
host cancels any active work and drops provider continuation state so later
requests cannot see removed history. `createChat.initialMessage` immediately
starts its first user turn; source-based chats and per-chat workspaces remain
unsupported.

Chat attachments are resolved atomically when a message is accepted, including
queued and steering input. The host supplies a fixed retained transcript through
`endTurn` (or the latest finished turn) as upstream text, even across sessions.
Invalid source chats or bounds are rejected; empty transcripts are allowed.
Nested chat attachments remain references. Messages accept at most 16 chat
attachments, deduplicated by `(resource, endTurn)` before resolution, and at most
1 MiB of serialized materialized transcripts in total. Over-limit turns and
pending messages are rejected atomically. Materialized context stays pinned
through later source changes and is reused on subsequent conversation turns.

Tools registered in `SessionActiveClient.tools` are exposed to the model,
deduplicated in favor of the client starting the turn. Invocations retain their
client contributor from start through completion and wait in `running` for that
owner's result. Owner content updates replace live content; another client's
updates are rejected. Results, including structured content, reach both durable
state and the next model round. Owner removal/disconnect or a five-minute timeout
produces a failed result; turn cancellation and shutdown release waits promptly.
If a connected owner no longer registers a tool selected earlier, only that
invocation fails; its other running calls remain available for completion.

`chat/pendingMessageSet` and `chat/pendingMessageRemoved` implement AHP 1.0's
independent single `steeringMessage` slot and ordered `queuedMessages` list.
Steering sets replace the slot; queued sets upsert by ID without changing their
position. `chat/queuedMessagesReordered` affects queued input only. Queued input
starts when idle or after the current turn ends, with a removal followed by
`chat/turnStarted.queuedMessageId`. Steering is delivered at model-request
boundaries and retained while idle. Pending snapshot entries use the SDK's
`PendingMessage` type without a `kind` field. Chat timestamps and implicitly
cancelled tool calls match the published reducer actions exactly.

`AgentProfile` is the environment port. `WorkspaceAgentProfile` adapts
`workspace-agent-tools` for workspace selection, preflight, exact-command
approval identity, and cancellation-aware execution (including process-tree
termination supplied by that crate). It also exposes the same configured roots
for `resourceRead`, `resourceWrite`, `resourceList`, `resourceMkdir`,
`resourceDelete`, `resourceMove`, `resourceCopy`, `resourceResolve`, and
`resourceRequest`. Only local `file:` URIs are supported; canonical paths and
symlink targets must stay within a configured root. Resource access is already
granted within those roots, and `resourceRequest` cannot expand them. Custom
profiles deny resource access unless they implement `resource_workspaces`.

`resourceWrite` supports UTF-8/base64, byte-oriented truncate/append/insert,
exclusive creation, and etag preconditions. Host filesystem requests are
serialized so concurrent appenders cannot overwrite one another. Resource I/O
runs outside the action publication gate, and watcher inventories run on blocking
workers. Read, write, and copy operations reject non-regular files (including
FIFOs, devices, and sockets); Unix opens are non-blocking and recheck file kind
before reading, truncating, or copying.
`createResourceWatch` supplies immutable typed channel snapshots and notify-backed,
batched `resourceWatch/changed` actions with include/exclude globs. Watch actions
use the store's global sequence and replay journal. The last unsubscribe,
disconnected peer, or host shutdown releases the underlying watcher.

**Audience:** host authors adding AHP lifecycle, subscriptions, replay, and
turn orchestration without choosing an HTTP or WebSocket stack. Start at
`src/lib.rs`, then read `host.rs`, `turns.rs`, and `store.rs`. Next, read
[`ahp-server-ws`](../ahp-server-ws/README.md) for transport and
[`ahp-server-redb`](../ahp-server-redb/README.md) for durable transitions; the
repository [architecture](../../docs/architecture.md) shows their ordering.
