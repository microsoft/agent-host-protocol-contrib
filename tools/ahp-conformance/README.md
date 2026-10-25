# AHP conformance

A black-box [Agent Host Protocol](https://microsoft.github.io/agent-host-protocol/)
conformance CLI. It discovers the host's advertised agent/model and capabilities,
then exercises transport, sessions, chats, resources, tools, authentication,
terminals, automations, and telemetry.
Node **24+** runs the TypeScript directly; there is no build step.

Tests target the **released specification matching the installed SDK**, currently
AHP **1.0.0** (`@microsoft/agent-host-protocol` 1.0.0). Protocol HEAD is not the
test oracle. When authoring tests from a protocol checkout, use the matching
release tag/commit (for 1.0.0, `spec/v1.0.0` = `5f16d81b`) and verify
assertions, spec anchors, and MUST/SHOULD wording against that release.

Tests of **proposed**, not-yet-ratified behavior (for example windowed delivery)
live in the `Proposed` area and are skipped unless you pass `--proposed` or set
`run.proposed: true`. They negotiate the proposed protocol version (`1.1.0`, from
the protocol repository's `main` at the revision pinned in
`src/harness/proposed.ts`) and skip on hosts that don't implement the feature.

## Quick start

```sh
cd tools/ahp-conformance
npm install
node src/cli.ts --init
# Edit ahp-conformance.yml: set target.url to your host's AHP WebSocket endpoint.
node src/cli.ts
```

Without a config, an interactive terminal offers to create one and run it.
In CI, create a config explicitly with `--init` or supply a checked-in config:

```sh
node src/cli.ts ./my-host.yml --reporter cli --json ahp-conformance-results.json
node src/cli.ts --list
node src/cli.ts ./my-host.yml --filter 'transport/**' --filter 'sessions/**'
node src/cli.ts ./my-host.yml --reporter html --no-open
node src/cli.ts ./my-host.yml --proposed
```

Exit codes: **0** no failed tests (skips are fine), **1** test failures or an
interrupted test run, **2** configuration/startup/reporting error. Ctrl-C aborts
active tests, finishes reports, and stops any launched host process group.
SIGTERM and SIGHUP follow the same cleanup path; an exit backstop kills a
remaining launched host. Startup refuses an endpoint already answering pings.

## Configuration

The generated [commented template](src/config/template.ts) documents every option
and default, top to bottom. Its language-server header points at
[conformance.schema.json](conformance.schema.json); regenerate that schema with
`npm run schema` after changing the Zod schema.

- `target.url`: `ws://` or `wss://`; optional `headers` and `query` maps apply
  transport authentication.
- Secrets in headers, query values, and `auth.tokens[].token` accept a plain
  string, `{ value: ... }`, `{ env: NAME }`, `{ prompt: "Label" }` (hidden,
  interactive only), or `{ command: "gh auth token" }` (shell, trimmed stdout).
  Tokens are authenticated after each initialize. Secrets are redacted from
  frames/logs when at least six characters long; authenticate token fields are
  always hidden. Only explicitly configured query entries are secret sources,
  not query parameters embedded in `target.url`. Endpoint query values/userinfo
  are always hidden in the profile.
- Optional `host.command`, `cwd`, `env`, and `readyTimeoutMs` launch a shell
  command, await WebSocket ping readiness, and stop it after the run. The timeout
  defaults to 120 seconds. `${scratchDir}` expands in command/env values.
- `agent.provider` / `agent.model` select advertised ids; defaults are the first
  agent and its first model. The selected model is sent with user messages.
  Hosts that publish models only after authentication are awaited for up to
  `agent.catalogTimeoutMs` (default 30000).
- `model.enabled: false` skips model-backed tests. Set `model.deterministic: true`
  only for the deterministic-openai fixture, enabling fixture-only error tests.
- `quirks` opts into workarounds for known host deviations. A quirk only relaxes
  shared setup so one deviation fails just the tests that target it.
  `lazySessionInit: true` is for hosts whose sessions stay `creating` until their
  first turn: setup continues once a default chat exists, and tests that need an
  initialized session call `SessionHandle.ready()`, which then runs one short turn.
- `workspace.scratchDir` optionally selects a parent directory. Every run creates
  its own fresh `run-<timestamp>-<random>` subdirectory there; existing files and
  directories are never removed. Without this setting, a fresh directory is
  created under the OS temporary directory. The run directory is canonicalized
  with `realpath`, and each test gets an empty subdirectory. Only the newly
  created run directory is deleted unless `workspace.keep: true`. Scratch
  directories inside a git repository are refused, because hosts may create
  worktrees or branches for sessions working in a repository.
  `sharedFilesystem: false` skips tests requiring host access to runner-local paths.
- `run.concurrency` defaults to 4; `timeoutMs` to 30000. `include` / `exclude`
  accept test-id glob lists. `*` stays within one `/` segment; `**` crosses them.
  Repeated `--filter` values are ORed, then intersected with `include`, with
  `exclude` taking precedence. `proposed: true` (or `--proposed`) also runs the
  proposed-behavior tests.
- `report.reporter: auto` chooses HTML only when stdin/stdout are TTYs and `CI`
  is unset; otherwise CLI. `open` defaults to true. `htmlFile` defaults to
  `ahp-conformance-report.html`; `jsonFile` is optional. CLI flags override these.

Paths are relative to the config file, except the CLI `--json` path, which is
relative to the current working directory. Config files and secret commands are
trusted executable input; do not run untrusted configs.

### Local sample host

```sh
node src/cli.ts examples/advanced-workspace-host.yml
```

The [example](examples/advanced-workspace-host.yml) launches the repository's
Rust advanced workspace host on port 18787 with its internal deterministic
fixture. Cargo and the sample's frontend prerequisites must be installed; the
first build may require a larger `host.readyTimeoutMs`.

### VS Code agent host

```sh
export AHP_VSCODE_TOKEN="$(openssl rand -hex 24)"
node src/cli.ts examples/vscode-agent-host.yml
```

The [example](examples/vscode-agent-host.yml) launches a separate, isolated
`code-insiders agent host --new-instance` on port 18898 (never an editor's agent
host), authenticates with `gh auth token`, and runs real model turns with the
Copilot agent. See the [VS Code agent host report](reports/vscode-agent-host.md)
for its current results and the deviations they reveal.

## Reports and skips

CLI mode prints a line per test, area totals, and failure details/logs. HTML mode
opens a live loopback report and saves a self-contained final HTML file, plus a
one-line CLI summary. `--json file` adds a structured report with the profile,
all selected tests, statuses, errors, durations, and logs, plus the redacted wire
transcript of each failed test.

`--list` shows stable ids and titles grouped by area without opening a connection.
A missing advertised capability is **skipped**, not failed:
`Host does not advertise <label>`. A disabled/unconfigured prerequisite is
`Disabled in config: <label>`. Tests may also discover a runtime reason to skip.
Optional 1.0.0 features without a capability flag are probed: for example,
terminal tests skip when `createTerminal` returns `MethodNotFound`.

## Adding tests

Export `TestDefinition` entries from the appropriate `src/tests/<area>/index.ts`.
Use ids like `resources/read/not-found`, an area/feature/title/spec anchor, and
`async run(t)`. Add explicit `requires` from `src/harness/requirements.ts` rather
than keying on provider names. Use `t.client()`, `t.session(client)`, mirror/action
waiters, `t.defer()` cleanup, and `src/harness/assert.ts` for assertions. Call
`session.ready()` before asserting behavior that needs an initialized session
(catalog entries, root notifications, `createChat`). Use the
shared prompts in `src/harness/prompts.ts` for model tasks; fixture-only directives
must require `requires.deterministicModel`. Every frame is recorded automatically.

Development: `npm run typecheck`, `npm test`. No emitted JavaScript is needed.
