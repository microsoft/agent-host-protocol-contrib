# Agent Host Protocol samples

Learn [Agent Host Protocol (AHP)](https://github.com/microsoft/agent-host-protocol)
progressively. Start with the small text host; move to the persistent workspace
host only when its additional behavior is useful.

## Start here

[`minimal-chat-host`](samples/rust/minimal-chat-host/README.md) is the first
sample to read and run. It is about 800 lines of production Rust with one
fixed in-memory session and chat. It teaches AHP initialization, subscription,
`chat/turnStarted`, and streamed text from the OpenAI **Responses API**. It has
no replay, tools, persistence, or static UI.

```bash
cargo run -p deterministic-openai -- --bind 127.0.0.1:8899

# In another terminal:
cargo run -p minimal-chat-host -- \
   --openai-base-url http://127.0.0.1:8899 \
  --openai-model fixture-model

# Alternatively, use a real OpenAI model, or anything compatible:
OPENAI_API_KEY=your-api-key cargo run -p minimal-chat-host -- \
   --openai-base-url https://api.openai.com/v1 \
   --openai-model gpt-5.6-luna
```

The fixture remains the deterministic option for tests and offline learning.

## Learning sequence

1. **[`minimal-chat-host`](samples/rust/minimal-chat-host/README.md)** —
   AHP plus one Responses-streamed text turn.
2. **[`ahp-server-core`](crates/ahp-server-core/README.md)** and
   **[`ahp-server-ws`](crates/ahp-server-ws/README.md)** — subscriptions,
   reconnect/replay, ordered actions, and the rules for committing a state
   transition before publishing it. Add
   **[`ahp-server-redb`](crates/ahp-server-redb/README.md)** when persistence
   is needed.
3. **[`advanced-workspace-host`](samples/rust/advanced-workspace-host/README.md)** —
   the advanced composition sample: redb, workspace tools and approvals, both
   OpenAI APIs, and optional static frontend hosting.
4. **[`ahp-chat`](samples/web/ahp-chat/README.md)** — a browser client that
   connects, subscribes, reconnects, and renders streamed text and tool
   approvals.

## What is reusable

The advanced host is assembled from small crates, each with a concrete job:

| Crate | Job |
| --- | --- |
| [`ahp-server-core`](crates/ahp-server-core/README.md) | AHP lifecycle, turns, subscriptions, replay rules, and the store/profile interfaces. |
| [`ahp-server-ws`](crates/ahp-server-ws/README.md) | Axum WebSocket endpoint and browser Origin policy. |
| [`ahp-server-redb`](crates/ahp-server-redb/README.md) | redb state, replay journal, recovery, and persisted exact-command approvals. |
| [`openai-agent-model`](crates/openai-agent-model/README.md) | OpenAI-compatible Responses and Chat Completions SSE streaming. |
| [`deterministic-openai-fixture`](crates/deterministic-openai/README.md) | Deterministic OpenAI-shaped HTTP/SSE behavior for tests. |
| [`workspace-agent-tools`](crates/workspace-agent-tools/README.md) | Workspace selection, file replacement, shell execution, and tool definitions. |

The core does not know about WebSockets, redb, HTTP model requests, browser
assets, or filesystem commands. The advanced host wires those crates together;
only it hosts the built static UI. Static hosting is not core AHP behavior.
See the [architecture](docs/architecture.md) for the flow and boundaries.

## Developer reading path

Read a sample's `main.rs` or `lib.rs` first. Follow one text turn end to end,
then one reconnect transition in the reusable server layer. Read workspace
tools only when the host needs them. The detailed file-by-file route is in
[`docs/reading-path.md`](docs/reading-path.md).

## Fastest full-stack demo

Run the advanced host with its internal deterministic model fixture:

```powershell
cargo run -p advanced-workspace-host -- --demo
```

Open the printed `Chat UI` URL and click **Connect**. The command builds the
browser app only when it is missing (or when `--rebuild-web` is supplied),
starts a loopback deterministic fixture, and serves both the UI and `/ahp`.

## Run the advanced sample with a configured model

For a real OpenAI-compatible endpoint, build the optional browser UI yourself
if needed, then supply the model. The default base URL is the production
OpenAI origin and automatic API mode tries Responses first:

```powershell
Push-Location samples\web\ahp-chat
npm ci
npm run build
Pop-Location

$env:OPENAI_API_KEY = "your-api-key"
cargo run -p advanced-workspace-host -- --openai-model gpt-5.6-luna
```

The host prints its exact Chat UI and AHP URLs. Browser WebSocket upgrades are
same-origin by default; native clients without an `Origin` header are accepted.
A separate Vite origin must be explicitly allowed. Configuration, timeout,
tool, and security details are in the
[advanced host guide](samples/rust/advanced-workspace-host/README.md).
Set `OPENAI_BASE_URL` or pass `--openai-base-url` to use another compatible
endpoint; the explicit value overrides the production default.

## Test tools and validation

[`deterministic-openai`](tools/deterministic-openai/README.md) supplies stable
v1 text/error and v2 sequential-tool contracts. The
[`ahp-conformance`](tools/ahp-conformance/README.md) runner distinguishes
protocol-core, text-chat, local fixture, workspace-tool, and model-streaming
scopes.

Run Rust checks once from the repository root, then frontend and plugin checks:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

pushd samples/web/ahp-chat
npm ci
npm run lint
npm test
npm run build
popd

python -c 'import json; json.load(open("plugins/ahp-development/plugin.json"))'
find plugins/ahp-development/skills -mindepth 1 -maxdepth 1 -type d -exec test -f '{}/SKILL.md' \; -print
```

For an end-to-end local stack, run:

```powershell
cargo run -p ahp-conformance -- --local-fixture
```
