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

# Alternatively, use production OpenAI with the sample defaults:
OPENAI_API_KEY=your-api-key cargo run -p minimal-chat-host
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

```bash
cargo run -p advanced-workspace-host -- --demo
```

Open the printed `Chat UI` URL and click **Connect**. The command builds the
browser app only when it is missing (or when `--rebuild-web` is supplied),
starts a loopback deterministic fixture, and serves both the UI and `/ahp`.

## Run the advanced sample with production OpenAI

For a real OpenAI-compatible endpoint, build the optional browser UI yourself
if needed. The omitted base URL selects the production OpenAI origin, so an API
key is required. The host discovers the model catalog and the user chooses a
model in the Chat UI; automatic API mode tries Responses first:

```powershell
Push-Location samples/web/ahp-chat
npm ci
npm run build
Pop-Location

$env:OPENAI_API_KEY = "your-api-key"
cargo run -p advanced-workspace-host
```

```bash
(cd samples/web/ahp-chat && npm ci && npm run build)

OPENAI_API_KEY=your-api-key cargo run -p advanced-workspace-host
```

The host prints its exact Chat UI and AHP URLs. Browser WebSocket upgrades are
same-origin by default; native clients without an `Origin` header are accepted.
A separate Vite origin must be explicitly allowed. Configuration, timeout,
tool, and security details are in the
[advanced host guide](samples/rust/advanced-workspace-host/README.md).
Set `OPENAI_BASE_URL` or pass `--openai-base-url` to use another compatible
endpoint; the explicit value overrides the production default and does not
require a key. `--openai-model` remains an optional host default or fallback
for endpoints without a model catalog.

## Test tools and validation

The TypeScript tools are independently published npm packages:

| Package | Executable | Guide |
| --- | --- | --- |
| `@microsoft/agent-host-protocol-cli` | `ahp` | [Host inspection, capture/replay, and persistent session controllers](tools/ahp-cli/README.md) |
| `@microsoft/agent-host-protocol-conformance` | `ahp-conformance` | [Black-box protocol conformance tests](tools/ahp-conformance/README.md) |

Both start at version `0.0.1`. After publication:

```bash
npm install --global @microsoft/agent-host-protocol-cli @microsoft/agent-host-protocol-conformance
ahp describe
ahp-conformance --init
```

The CLI requires Node 22+; conformance requires Node 24+.
See [RELEASING.md](RELEASING.md) for the Azure DevOps publishing pipelines.

[`deterministic-openai`](tools/deterministic-openai/README.md) supplies
versioned text/error and tool fixtures, including v4 recall/read-file/write-file
directives. The Node 24+ TypeScript
[`ahp-conformance`](tools/ahp-conformance/README.md) CLI runs granular black-box
WebSocket JSON-RPC probes across Transport & lifecycle, Root & sessions,
Chats & turns, Resources, Tools & permissions, and Authentication. Tests for
capabilities the host does not advertise are skipped; model-backed probes use
natural-language prompts with embedded `[AHP-TEST vN]` directives for real models
and the deterministic fixture. See its README for configuration and reports.

Run Rust checks once from the repository root, then frontend and plugin checks:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked

pushd samples/web/ahp-chat
npm ci
npm run lint
npm test
npm run build
popd

python -c 'import json; json.load(open("plugins/ahp-development/plugin.json"))'
find plugins/ahp-development/skills -mindepth 1 -maxdepth 1 -type d -exec test -f '{}/SKILL.md' \; -print
```

## Continuous integration and merge requirements

[GitHub Actions CI](.github/workflows/ci.yml) runs on every pull request, push to
`main`, and merge-queue group. It has no path filters, so documentation-only PRs
also produce the check required for merging.

| Job | Checks |
| --- | --- |
| Rust | rustfmt, Clippy with warnings denied, compilation of all targets/features, workspace and documentation tests. |
| Web sample | Existing Oxlint rules, Vitest tests, TypeScript compilation, and Vite production build on Node 24. |
| TypeScript tools | Typechecks, builds, tests, and installed npm-artifact checks; CLI on Node 22/24 and conformance on Node 24. |
| Plugin metadata | Valid JSON manifest and a `SKILL.md` entrypoint for every skill. |

The final **Required CI** job runs even when another job fails. It succeeds only
when every validation job succeeds; failed, cancelled, or skipped dependencies
block it. Existing frontend lint warnings remain warnings under its checked-in
Oxlint configuration. No new JavaScript linter is imposed on the CLI or conformance
suite; their existing strict TypeScript checks run instead.

Workflow YAML alone cannot require checks before merging. A repository
administrator must apply the [main-branch ruleset](.github/rulesets/main.json):

```sh
gh api --method POST repos/microsoft/agent-host-protocol-contrib/rulesets \
  --input .github/rulesets/main.json
```

The ruleset requires pull requests, an up-to-date branch, and the **Required CI**
check from the GitHub Actions app. It does not require reviewer approvals.
Apply it once; if the repository already has equivalent rules, update those
instead of creating duplicates. The Azure DevOps npm publishing pipelines remain
manual-only and are separate from these merge checks.

For an end-to-end local stack with Node 24+ (no build step), run:

```bash
cd tools/ahp-conformance
npm install
node src/cli.ts examples/advanced-workspace-host.yml
```

This config launches `advanced-workspace-host --demo`, which embeds the
deterministic fixture. `npm start -- …` is also supported.
