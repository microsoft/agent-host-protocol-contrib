# Developer reading path

Use this order to understand one complete behavior before reading the next
layer. The paths below are relative to the repository root.

## 1. Minimal text turn

1. Start at
   [`samples/rust/minimal-chat-host/src/main.rs`](../samples/rust/minimal-chat-host/src/main.rs).
2. Read [`host.rs`](../samples/rust/minimal-chat-host/src/host.rs) for fixed
   runtime state, the WebSocket connection loop, and action fanout.
3. Read [`protocol.rs`](../samples/rust/minimal-chat-host/src/protocol.rs) for
   initialization, subscriptions, snapshots, actions, and action rejections.
4. Read [`responses.rs`](../samples/rust/minimal-chat-host/src/responses.rs)
   for the Responses request and SSE decoding.
5. Follow one `chat/turnStarted` through `chat/responsePart`,
   `chat/delta`, and `chat/turnComplete`.
6. Read the sample README's omissions before moving on. There is no reconnect,
   journal, model history, tool, or static UI path to find here.

## 2. Persistent text turn and reconnect

1. Start the advanced composition root at
   [`samples/rust/advanced-workspace-host/src/main.rs`](../samples/rust/advanced-workspace-host/src/main.rs),
   then [`src/lib.rs`](../samples/rust/advanced-workspace-host/src/lib.rs).
2. Follow a turn in
   [`crates/ahp-server-core/src/turns.rs`](../crates/ahp-server-core/src/turns.rs)
   and command handling in
   [`src/host.rs`](../crates/ahp-server-core/src/host.rs).
3. Follow its committed action envelopes through the `HostStore` definitions in
   [`src/store.rs`](../crates/ahp-server-core/src/store.rs), then the redb
   implementation's
   [`src/sessions.rs`](../crates/ahp-server-redb/src/sessions.rs) and
   [`src/replay.rs`](../crates/ahp-server-redb/src/replay.rs).
4. For one reconnect, return to core command handling, then read
   [`ahp-server-ws/src/lib.rs`](../crates/ahp-server-ws/src/lib.rs) to see how
   a WebSocket becomes a logical peer. The browser counterpart is
   [`samples/web/ahp-chat/src/ahp`](../samples/web/ahp-chat/src/ahp).

## 3. Tools only when needed

After the text and reconnect paths are clear, read
[`workspace-agent-tools/src/lib.rs`](../crates/workspace-agent-tools/src/lib.rs),
then `workspace.rs`, `replace_string.rs`, and `shell.rs`. Return to
`ahp-server-core/src/profile.rs` to see where the core requests workspace
preflight and execution. The tool approval and safety limits are described in
the [advanced host guide](../samples/rust/advanced-workspace-host/README.md).

## 4. Model adapter and browser client

For API selection and streaming conversion, start at
[`openai-agent-model/src/lib.rs`](../crates/openai-agent-model/src/lib.rs), then
`client.rs`, `responses.rs`, and `chat.rs`. For the client, start at
[`samples/web/ahp-chat/src/main.tsx`](../samples/web/ahp-chat/src/main.tsx),
then follow its connection manager and store under `src/ahp`.

Use the [architecture](architecture.md) for the boundaries and transition
diagrams, and the [conformance README](../tools/ahp-conformance/README.md) to
see the externally observable behavior.
