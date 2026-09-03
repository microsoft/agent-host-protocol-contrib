# AHP Chat

A small browser-only React + TypeScript reference client for an
[Agent Host Protocol (AHP)](https://github.com/microsoft/agent-host-protocol) WebSocket host.
It is deliberately a chat sample, not an agent IDE: sessions, chats, prompts,
streamed assistant text, server-tool calls and approvals, cancellation, and
reconnection are included; plugins, MCP, resource browsing, terminal channels,
automations, and attachments are not.

## Run locally

```powershell
Set-Location samples/web/ahp-chat
npm install
npm run dev
```

```bash
cd samples/web/ahp-chat
npm install
npm run dev
```

The development connection form defaults to `ws://127.0.0.1:8787/ahp`.
Start the repository's `advanced-workspace-host` separately and allow the Vite origin,
then click **Connect**:

```powershell
cargo run -p advanced-workspace-host -- --allowed-origin http://localhost:5173
```

If Vite prints a different origin, pass that exact `http://` or `https://`
origin instead. Host-served production builds are same-origin and need no
override.

```powershell
npm test
npm run lint
npm run build
```

```bash
npm test
npm run lint
npm run build
```

`npm run build` emits the static site to `dist`. The repository's
`advanced-workspace-host` can serve that directory at `/` and keeps its
WebSocket endpoint at `/ahp`. Static hosting belongs to that advanced
composition sample, not to AHP or the reusable server crates. If `dist` is
missing, the host remains available and its HTTP root shows the exact frontend
build commands.

## Workspaces and tool approvals

When creating a session, the optional **New session workspace** field accepts an
absolute Windows path (such as `C:\work\project`), an absolute POSIX path, or a
`file:` URI. The browser normalizes it to a `file:` URI but does not check
whether it exists. That path is interpreted by the connected **host machine**,
not necessarily by the machine running the browser. Leave it empty to let the
host choose its default; the most recently successful non-empty input is kept
in browser local storage.

Tool calls are shown inline with assistant response parts. File replacement
tools can be auto-approved by the host. Shell calls require an explicit choice
when the host asks: **Allow once**, **Always allow…**, or **Deny**. “Always
allow” is a host-side persisted setting for the exact command string, not every
invocation of that executable. Shell commands run on the host and their working
directory is not a sandbox.

## Chat drafts, models, and reasoning

Sessions are created with only their optional workspace. The model picker lives
beside the chat input, using the root snapshot's advertised model catalogue and
per-model reasoning schema; names, IDs, labels, and descriptions come from the
host. Changing a model or reasoning level updates that chat's draft, not the
session configuration.

Draft text and its model selection are shared, persisted `ChatState.draft`
state. The UI debounces `chat/draftChanged` while typing, restores authoritative
drafts after reconnecting or switching chats, and flushes the latest draft
before `chat/turnStarted`. Draft messages include a sample-specific expected-turn
value in `Message._meta`; the sample host strips it before persistence and
rejects stale writes after another client starts a turn. The current session
default or a valid locally remembered selection seeds a new empty chat. The
renderer specializes `thinkingLevel` while preserving other primitive model
configuration fields.

## Choosing the WebSocket URL

`src/ahp/runtime-config.ts` is the sole URL boundary. It resolves, in order:

1. `window.__AHP_SAMPLE_CONFIG__?.webSocketUrl`, if supplied by the hosting page.
2. The last successfully connected address from `localStorage` (unless runtime
   configuration was supplied).
3. Same-origin `ws://` or `wss://` plus `/ahp` in a static production host.
4. The local mock-host address during Vite development.

For example, a static host can place this before the Vite bundle:

```html
<script>
  window.__AHP_SAMPLE_CONFIG__ = { webSocketUrl: "wss://example.test/ahp" };
</script>
```

## Data flow

`ConnectionManager` opens `WebSocketTransport`, creates `AhpClient`, calls
`initialize` with the package's `SUPPORTED_PROTOCOL_VERSIONS`, and attaches the
root subscription before the handshake. `AhpService` keeps protocol command and
action names in `src/ahp`, fetches the session catalogue, and selects/subscribes
to session and chat resources on demand. Its client ID is stored in
`sessionStorage`: it survives reconnects and reloads in one tab, but is distinct
from IDs in other tabs. Before connecting, a short `BroadcastChannel` claim
detects duplicated opener-tab storage and rotates the duplicate's candidate.
If channel or storage support is unavailable, the client uses a page-lifetime
in-memory ID to avoid a collision; that fallback does not persist across reloads.

`AhpStore` uses the SDK's `AhpStateMirror` for root/session state and the SDK's
pure `chatReducer` for chat actions. It projects those internal protocol states
to fresh, frozen UI snapshots for `useSyncExternalStore`; React never consumes
the mirror's mutable maps. Sequence numbers, connection epochs, subscription
closure, and async-iterator teardown prevent an old socket from updating a new
connection. A dropped established socket reconnects with the SDK's typed
`reconnect` call and applies either replay actions or replacement snapshots.

Session and chat selection has its own epoch, independent of the connection
epoch. A delayed selection can load state, but cannot select its default chat
after a newer click wins. `SubscriptionRegistry` reconciles subscriptions on
every selection to exactly `ahp-root://`, the selected session, and the selected
chat. It closes local consumers and sends typed `unsubscribe` notifications for
anything else; that compact set is also the reconnect scope. Rechecks around
awaits prevent overlapping selections or a replaced connection from dropping a
newly needed resource.

## shadcn/ui chat primitives

The current shadcn CLI generated the components in `src/components/ui`:

```powershell
npx shadcn@latest add message-scroller message bubble marker
```

The conversation uses `MessageScrollerProvider`, `MessageScrollerViewport`,
`MessageScrollerItem`, and `MessageScrollerButton`; generated viewport styles
include shadcn's `scroll-fade-b` utility. It composes the generated `Message`,
`Bubble`, and `Marker` APIs for user turns, assistant deltas, stream state, and
turn errors. The June 2026 registry exposes these as composed subcomponents
rather than invented one-shot props, so this sample follows the generated API.
`Attachment` was intentionally not added because the target host explicitly
rejects attachment/resource surfaces.

## Read next

**Audience:** client developers learning an SDK-backed AHP connection, state
projection, and reconnect UI. Start at `src/main.tsx`, then follow
`src/ahp/runtime-config.ts`, `ConnectionManager`, and `AhpStore`. Pair it with
the repository [reading path](../../../docs/reading-path.md) and the
[advanced host guide](../../rust/advanced-workspace-host/README.md).
