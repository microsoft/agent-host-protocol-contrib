# ahp-server-redb

`RedbHostStore` is the durable `ahp_server_core::HostStore` implementation.
It owns host session/chat state, the bounded action replay journal, and
exact-command approval keys; models and workspace tools remain core concerns.

Every `HostStore` operation that returns `CommittedTransition` changes records
and appends its contiguous `ActionEnvelope`s in **one redb write transaction**.
Callers may publish only the returned envelopes. Failed lifecycle transitions
do not change state or advance the server sequence.

Replay is retained for 2,048 envelopes; older cursors return
`ActionReplay::SnapshotRequired`. Chat history retains the latest 256 completed
turns. Both limits intentionally match the existing host behavior and are
fixed rather than configuration surface.

Chat and session summaries follow AHP 1.0 status reduction: a pending tool or
result confirmation reports `InputNeeded`, and a session promotes blocked or
error state from any child chat even when its newest/default chat is idle.

Session titles and independent read/archive bits survive database reopen.
Chat read flags are persisted independently from session and sibling flags;
starting a turn clears the chat's read bit. Session catalogs include the chat's
working directories and root summaries expose the complete ordered compact
catalog with exact statuses and the default chat.

Tool confirmations and client-tool execution requests are mirrored into the
session input queue in the same transaction as their chat state and journal.
Upserts preserve input queue order; resolution, cancellation, truncation, and
restart recovery journal removals alongside the corresponding chat actions.
Catalog reconciliation loads only sessions addressed by catalog-affecting actions;
text/tool-input deltas and non-catalog mutations skip reconciliation entirely.
Session disposal journals one `session/chatRemoved` per catalog entry before
the root active-session count update.
Active-client membership is committed with its action envelopes, but restart
recovery clears disconnected clients and journals their removals atomically
with interrupted-turn recovery. Older records default to no metadata flags or
active clients. Optional activity strings are consistently omitted from chat
and session projections.

AHP 1.0's single steering-message slot and independent ordered queue survive
restart. Steering replacement, queued upserts, kind-specific removals, queue
reorders, and history truncation commit atomically with
their replay actions. Starting a queued turn atomically removes its pending
entry and journals removal before `chat/turnStarted`. Truncation also drops
provider continuation state. Terminal `modifiedAt` is derived from the recorded
`startedAt` plus the journaled duration, including restart recovery; implicit
tool cancellation preserves input without inventing a reason message.

Chat attachment transcripts are materialized in the same transaction that accepts
the containing turn or pending message. Pinned context is persisted separately
from wire messages, survives reopen and source pruning, and never recursively
expands nested chat references. Messages are limited to 16 chat attachments and
1 MiB of total serialized transcript context, with duplicate resource/bound pairs
materialized only once; rejection commits no records or replay actions. Legacy
records default to no materialized context.
Client-tool live-content replacements and completions validate execution ownership
inside their write transaction and journal the matching action atomically,
including structured results. Interrupted client calls follow ordinary turn
recovery and cannot leave a resumed process waiting on a disconnected executor.

Existing JSON session/chat records remain readable, including old turns that
store `response` without `response_parts`. The legacy
`shell_approved_commands` metadata is deliberately ignored: only
`exact_command_approvals` keys made from `ExactCommandApproval` are honored,
so historical executable-wide grants cannot broaden authorization.

`open` recovers active turns left by a previous process, terminalizes incomplete
tool calls, and journals the resulting `chat/error` and session summary actions
atomically. `close` is optional and releases the redb handle early for Windows
file cleanup; dropping the store does the same.

**Audience:** host authors who need durable state, replay, recovery, and
exact-command approvals. Start at `src/lib.rs`, then follow `sessions.rs`,
`replay.rs`, and `recovery.rs`. Next, see
[`ahp-server-core`](../ahp-server-core/README.md) for the store calls and the
[architecture](../../docs/architecture.md) for the atomic fanout sequence.
