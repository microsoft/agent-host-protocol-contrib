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

Chat and session summaries follow AHP 0.8 status reduction: a pending tool or
result confirmation reports `InputNeeded`, and a session promotes blocked or
error state from any child chat even when its newest/default chat is idle.

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
