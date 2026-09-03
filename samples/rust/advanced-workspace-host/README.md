# Advanced workspace host

`advanced-workspace-host` is an **advanced composition sample**, not another
framework. Its focused composition modules use these reusable crates:

- `ahp-server-core` — AHP lifecycle, turn orchestration, bounded peer queues,
  and the `HostStore` contract.
- `ahp-server-redb` — durable redb `HostStore` adapter.
- `ahp-server-ws` — `/ahp` WebSocket transport and browser-origin policy.
- `openai-agent-model` — OpenAI-compatible Responses/Chat Completions
  streaming adapter.
- `deterministic-openai-fixture` — optional internal OpenAI-compatible demo
  fixture.
- `workspace-agent-tools` — canonical workspace selection and confined file
  and shell tools.

Only this sample owns Vite static hosting. Core and WebSocket crates remain
unaware of browser assets.

**Audience:** developers who already understand the minimal streamed text turn
and now need persistence, reconnect/replay, tools, or a browser UI. Read
[`src/cli.rs`](src/cli.rs), then [`src/lib.rs`](src/lib.rs), and follow the
repository [reading path](../../../docs/reading-path.md). For the component
boundaries and commit-before-fanout rule, read the
[architecture](../../../docs/architecture.md) next.

## Run

The fastest complete local stack is one command:

```powershell
cargo run -p advanced-workspace-host -- --demo
```

```sh
cargo run -p advanced-workspace-host -- --demo
```

It starts an internal loopback `deterministic-openai-fixture`, uses
`fixture-model` with automatic API selection, discovers its catalog (including
the fixture fast and reasoning choices), and prints the browser and AHP URLs.
Open the printed **Chat UI** URL, then click **Connect**; connecting is always
an explicit browser action.

The demo builds `samples/web/ahp-chat` only if its `dist/index.html` is
missing or unusable. It runs `npm ci` only when `node_modules` is absent or
its lockfile is stale, then runs `npm run build` with visible output. Node/npm
are therefore needed only for a missing build or `--rebuild-web`. Use
`--rebuild-web` to force that build. Demo rejects `--no-web`; ordinary
configured mode retains the 503 setup page when the frontend is absent. Demo
always serves the sample React build, so it also rejects an explicit
`--web-root`.

In ordinary mode, omitting `--openai-base-url` means
`https://api.openai.com`; that implied production endpoint requires
`OPENAI_API_KEY` or `--openai-api-key` before startup. The model is optional:
when discovery succeeds without `--openai-model`, select a discovered model in
the Chat UI before starting a session.

```powershell
$env:OPENAI_API_KEY = "your-api-key"
cargo run -p advanced-workspace-host -- `
  --database .ahp-samples/advanced.redb
```

```sh
export OPENAI_API_KEY="your-api-key"
cargo run -p advanced-workspace-host -- \
  --database .ahp-samples/advanced.redb
```

Set `OPENAI_BASE_URL` or pass `--openai-base-url` to use a compatible endpoint
instead. Supplying any base URL explicitly, including the production URL,
makes the API key optional. Use `--openai-model` for a host default/fallback;
for example, this supports a catalog-less compatible endpoint:

```powershell
cargo run -p advanced-workspace-host -- `
  --openai-base-url http://127.0.0.1:11434/v1 `
  --no-discover-models `
  --openai-model compatible-model
```

```sh
cargo run -p advanced-workspace-host -- \
  --openai-base-url http://127.0.0.1:11434/v1 \
  --no-discover-models \
  --openai-model compatible-model
```

The host serves the Vite build at `/` and its WebSocket at `/ahp`; `/ahp`
takes precedence over the SPA fallback. If `dist/index.html` is unavailable,
the host still starts and `/` returns a 503 page with the build commands.
Use `--no-web` (or `AHP_NO_WEB`) to deliberately show that guidance.

`AdvancedHost::start(AdvancedHostConfig)` binds the server and returns
`RunningAdvancedHost`, which exposes `http_url`, `websocket_url`, and
graceful `shutdown`. Passing `127.0.0.1:0` is supported for local conformance
stacks and tests. Explicit `shutdown` stops the transport, drains every core
turn/model/tool/approval task, then closes redb so the database can
immediately be renamed, deleted, or reopened on Windows. More precisely, it
cancels and awaits Axum, calls `CoreHost::shutdown_and_wait`, then closes redb.
Dropping the handle only requests best-effort cancellation; callers needing
deterministic cleanup must await `shutdown`.

## Configuration

The CLI keeps the familiar flags and environment variables:

| CLI flag | Environment variable | Default |
| --- | --- | --- |
| `--bind` | `AHP_BIND` | `127.0.0.1:8787` |
| `--database` | `AHP_DATABASE` | `.ahp-samples/advanced.redb`; `.ahp-samples/advanced-demo.redb` with `--demo` |
| `--openai-base-url` | `OPENAI_BASE_URL` | unset; omitted configured mode implies `https://api.openai.com` (ignored by `--demo`) |
| `--openai-model` | `OPENAI_MODEL` | unset; optional host default/fallback |
| `--no-discover-models` | `AHP_NO_DISCOVER_MODELS` | `false`; configured mode only |
| `--openai-api-key` | `OPENAI_API_KEY` | required only when the production base URL is implied |
| `--openai-api` | `OPENAI_API` | `auto` |
| `--demo` | — | `false` |
| `--rebuild-web` | — | `false`; requires `--demo` |
| `--chunk-size` | `AHP_CHUNK_SIZE` | `16` |
| `--connect-timeout-seconds` | `OPENAI_CONNECT_TIMEOUT_SECONDS` | `10` |
| `--idle-timeout-seconds` | `OPENAI_IDLE_TIMEOUT_SECONDS` | `75` |
| `--allowed-origin` | `AHP_ALLOWED_ORIGIN` | none |
| `--workspace-root` | `AHP_WORKSPACE_ROOT` | current directory |
| `--reset-shell-approvals` | `AHP_RESET_SHELL_APPROVALS` | `false` |
| `--web-root` | `AHP_WEB_ROOT` | `samples/web/ahp-chat/dist`; configured mode only |
| `--no-web` | `AHP_NO_WEB` | `false` |

Repeat `--allowed-origin` or `--workspace-root`; their environment values
also accept commas. Browser upgrades are same-origin by default, while native
clients without an `Origin` header remain accepted. Additional allowed origins
are additive, never a replacement for same-origin protection.

After every successful start the CLI prints `AHP WebSocket: <ws URL>`. If a
usable frontend is present, it prints `Chat UI: <http URL>/`. Otherwise the
Chat UI line truthfully reports that it is unavailable and gives the same HTTP
URL for the 503 build-guidance page. It also prints either `Models: N
available; default <id>` or `Models: N available; no host default`, without
exposing credentials.

## Model discovery, selection, streaming, and timeouts

Before accepting clients, configured mode requests `GET /v1/models` from the
OpenAI-compatible endpoint and advertises every returned model in deterministic
ID order. If supplied, `--openai-model` is the host default and is inserted
into the catalog if the endpoint does not list it. If omitted, the host has no
default: select a discovered model in the Chat UI/session configuration before
starting a turn. The model ID is used as its label when the endpoint has no
display-name metadata.

Each advertised model offers a production-inspired `thinkingLevel` configuration
whose choices depend on the selected model. Omit `thinkingLevel` (`Model
default`) to leave reasoning behavior to that model; the host never sends a
synthetic “default” value. A session selection supplies future turns, while a
per-message selection applies only to that message.

The catalog retains every valid discovered ID. Reasoning capabilities come from
an explicit table of traced full model IDs; names are never inferred from a
substring or prefix. Unknown compatible models remain selectable, but omit the
reasoning control until their exact ID is added to that table. The built-in
fixture demonstrates distinct fast, general, and extended-reasoning choices.

An endpoint responding to model discovery with 404, 405, or 501 is explicitly
reported at startup and falls back to `--openai-model` when one was supplied.
Without that fallback, startup fails and explains to use a compatible catalog
or `--openai-model`. Other HTTP responses (including authentication and
rate-limit failures), network errors, timeouts, and malformed catalog responses
fail startup. `--no-discover-models` requires `--openai-model` and advertises
only that model. The demo always discovers its three fixture models and uses
`fixture-model` as its default.

`--openai-api auto` tries Responses first and caches its selection. It falls
back to Chat Completions only when the initial Responses request returns HTTP
404, 405, or 501. A complete Responses or Chat Completions URL selects that
endpoint directly. Authentication, validation, rate-limit, network, server,
malformed-stream, and midstream failures never cause fallback.

When `--openai-base-url` is omitted, production OpenAI Responses requests use
`previous_response_id`. The final successful ID is stored per AHP chat in redb,
so later turns send only their new input and retain context across host
restarts. Explicit compatible endpoints and Chat Completions receive the
durable AHP user/assistant history instead. OpenAI still counts prior chained
tokens toward input usage.

Text and tool argument deltas stream through the model adapter and are
persisted and forwarded as AHP actions as they arrive. The connect timeout
covers connecting and receiving initial headers. The idle timeout is reset by
each non-empty response-body chunk. There is deliberately **no total stream
deadline**, so healthy long-running streams remain active.

The deterministic demo fixture returns stable Responses and Chat Completions
SSE behavior for ordinary prompts and its documented AHP test directives. It
is a CLI convenience only: shutdown first drains the advanced host and closes
redb, then stops the internal fixture.

## Workspaces, shell tools, and approvals

Workspace roots are canonicalized with `ConfiguredWorkspaces` before the core
starts. With no configured root, the process current directory is the sole
root. Sessions select zero or one canonical `file:` URI; selected locations
must exist, be directories, and remain under a configured root after symlink
resolution.

`replace_string` and `run_shell` are exposed as server tools. Shell commands
run with the selected workspace as their working directory, but that is **not
an OS sandbox**: a command may access paths outside it. Treat model-supplied
commands as untrusted.

An always-allow decision applies to one **exact, trimmed command string** and
is persisted host-wide in redb. It is **not an executable allowlist**, so
approving `git status` does not approve `git` or another command beginning
with it. Use `allow-once`, always allow that exact command, or deny as the
confirmation action requires. `--reset-shell-approvals` clears every persisted
exact-command approval at startup.

## Validate

Run the integrated sample checks:

```powershell
cargo fmt --check
cargo clippy -p advanced-workspace-host --all-targets --all-features -- -D warnings
cargo test -p advanced-workspace-host
```

Then run the black-box [conformance runner](../../../tools/ahp-conformance/README.md)
with `cargo run -p ahp-conformance -- --local-fixture`.
