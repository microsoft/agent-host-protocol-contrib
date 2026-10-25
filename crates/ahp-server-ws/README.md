# ahp-server-ws

Thin Axum WebSocket transport for `ahp-server-core`. `router` exposes only
`GET /ahp`: each upgrade obtains a bounded logical core peer, forwards inbound
text frames to `CoreHost::handle_text`, and sends the peer's outbound JSON-RPC
text frames. Binary, ping, and pong frames are ignored; close, shutdown, and
send/receive errors release the peer.
After an unsupported-version error, the adapter drains queued responses and
sends a WebSocket close frame.

The adapter owns the browser Origin policy: no `Origin` is accepted for native
clients, a browser Origin matching `Host` is accepted, and `WsOptions` adds
explicit HTTP(S) origins. Stores, models, static files, and workspace tools
remain outside this crate.

**Audience:** host authors attaching `ahp-server-core` to Axum. Read
`src/lib.rs`, then `src/origin.rs`; next read
[`ahp-server-core`](../ahp-server-core/README.md) for peer lifecycle or the
[advanced host](../../samples/rust/advanced-workspace-host/README.md) for a
complete composition. Static UI hosting is intentionally not part of this
transport.
