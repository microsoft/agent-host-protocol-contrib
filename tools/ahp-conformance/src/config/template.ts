import { writeFile } from 'node:fs/promises';
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

export function configTemplate(configPath: string, url = 'ws://127.0.0.1:8787/ahp'): string {
  const configDir = dirname(resolve(configPath));
  const schemaPath = fileURLToPath(new URL('../../conformance.schema.json', import.meta.url));
  const repositoryRoot = fileURLToPath(new URL('../../../../', import.meta.url));
  const fromRepository = relative(repositoryRoot, configDir);
  const insideRepository = fromRepository !== '..' && !fromRepository.startsWith(`..${sep}`) && !isAbsolute(fromRepository);
  const schema = (insideRepository ? relative(configDir, schemaPath) : schemaPath).replaceAll('\\', '/');
  return `# yaml-language-server: $schema=${schema}
# Agent Host Protocol conformance configuration. Paths are relative to this file.
# A running host only needs target.url; all other settings below have defaults.

target:
  # Required AHP WebSocket endpoint (ws:// or wss://).
  url: ${JSON.stringify(url)}
  # Optional upgrade headers; secret values are removed from reports.
  # headers:
  #   Authorization: { env: AHP_AUTHORIZATION } # Value must include the scheme, e.g. "Bearer <token>".
  #   X-Api-Key: { value: "local-development-key" }
  # Secret alternatives (usable in headers, query, and auth.tokens):
  #   X-Api-Key: "plain-text-secret"
  #   X-Api-Key: { env: AHP_API_KEY }
  #   X-Api-Key: { prompt: "API key" } # Hidden input; requires a TTY.
  #   X-Api-Key: { command: "gh auth token" } # Shell command, trimmed stdout.
  # Optional query parameters; use env sources rather than embedding credentials.
  query: {} # Default: no added query parameters.
  # query:
  #   access_token: { env: AHP_ACCESS_TOKEN }

# Optional host process. Omit this section to test an already running host.
# host:
#   command: 'node server.js --workspace "\${scratchDir}"' # Shell command; scratchDir is substituted.
#   cwd: "." # Default: the config file's directory.
#   env: # Extra child environment (inherits the current environment).
#     WORKSPACE: "\${scratchDir}" # Substitution also works in env values.
#   readyTimeoutMs: 120000 # Default: allow two minutes for a ping response.

# Optional AHP agent authentication, sent after every initialize.
# auth:
#   tokens: # Default: []; resources must be advertised by the host.
#     - resource: "https://api.github.com"
#       token: { command: "gh auth token" }
#     - resource: "https://api.example.com"
#       token: { env: EXAMPLE_TOKEN }

agent:
  # provider: "copilot" # Optional exact provider id. Default: first advertised agent.
  # model: "gpt-4.1" # Optional exact model id. Default: the agent's first model.
  catalogTimeoutMs: 30000 # Default: wait for models that appear only after authentication.

model:
  enabled: true # Default: run model-backed tests; false skips them.
  deterministic: false # True ONLY with the deterministic-openai fixture.

# Opt-in workarounds for known host deviations. A quirk only relaxes shared test setup so one
# deviation fails just the tests that target it; those tests still run and report the failure.
quirks:
  lazySessionInit: false # True if new sessions stay "creating" until their first turn.

workspace:
  # scratchDir: "./.ahp-scratch" # Optional parent for a fresh run-<timestamp>-<random> directory.
  # Default: a fresh ahp-conformance-<timestamp>-<random> directory under the OS temp dir.
  sharedFilesystem: true # Default: host can access runner paths unchanged.
  keep: false # Default: remove only the newly created run directory; preserve its parent.

run:
  concurrency: 4 # Default: maximum number of simultaneously running tests.
  timeoutMs: 30000 # Default: per-test timeout (some tests override this).
  include: [] # Default: all ids; e.g. ["transport/**", "sessions/**"].
  exclude: [] # Default: none; * matches within a segment, ** crosses /.
  proposed: false # Default: skip tests of proposed, not-yet-ratified protocol behavior (CLI: --proposed).

report:
  reporter: auto # Default: html for interactive terminals outside CI, else cli.
  open: true # Default: open live HTML in the default browser.
  htmlFile: ahp-conformance-report.html # Default: saved next to this config.
  # jsonFile: ahp-conformance-results.json # Optional additive JSON results.
`;
}

export async function writeTemplate(path: string, url?: string): Promise<void> {
  await writeFile(path, configTemplate(path, url), { flag: 'wx' });
}
