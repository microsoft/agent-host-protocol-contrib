import { z } from 'zod';

// Resolved, validated configuration consumed by the runner and tests.
// The YAML schema, defaults, secret resolution, and template live alongside this type.

export interface ResolvedConfig {
  /** Absolute path of the loaded config file. */
  configPath: string;
  target: {
    /** WebSocket URL with any configured query parameters applied. */
    url: string;
    /** Extra HTTP headers sent on the WebSocket upgrade. */
    headers: Record<string, string>;
  };
  /** Present when the runner should launch the host under test. */
  host?: {
    /** Shell command; `${scratchDir}` is substituted. */
    command: string;
    cwd: string;
    env: Record<string, string>;
    readyTimeoutMs: number;
  };
  auth: {
    /** Tokens pushed with `authenticate` after every `initialize`. */
    tokens: { resource: string; token: string }[];
  };
  agent: {
    /** Agent provider id; defaults to the first advertised agent. */
    provider?: string;
    /** Model id; defaults to the agent's first model. */
    model?: string;
    /** How long to wait for the agent's model catalog to advertise the model after authentication. */
    catalogTimeoutMs: number;
  };
  model: {
    /** Whether tests that need a language model run. */
    enabled: boolean;
    /** The host's model is the deterministic-openai fixture, enabling fixture-only directives such as upstream errors. */
    deterministic: boolean;
  };
  /**
   * Opt-in workarounds for known host deviations, so one deviation fails only the tests that target it.
   * Each quirk only relaxes shared setup; tests asserting the deviated behavior still run and fail.
   */
  quirks: {
    /** The host keeps new sessions `creating` until their first turn, so setup proceeds once a default chat exists. */
    lazySessionInit: boolean;
  };
  workspace: {
    /** Absolute local directory; each test gets its own subdirectory. */
    scratchDir: string;
    /** Whether the host sees `scratchDir` at the same path. */
    sharedFilesystem: boolean;
    /** Keep the scratch directory after the run. */
    keep: boolean;
  };
  run: {
    concurrency: number;
    /** Default per-test timeout. */
    timeoutMs: number;
    /** Glob patterns over test ids; empty means all. */
    include: string[];
    exclude: string[];
    /** Also run tests for proposed, not-yet-ratified protocol behavior (`requires.proposed`). */
    proposed: boolean;
  };
  report: {
    reporter: 'cli' | 'html';
    /** Open the HTML report in the default browser. */
    open: boolean;
    /** Final self-contained HTML report path. */
    htmlFile?: string;
    /** Machine-readable results path. */
    jsonFile?: string;
  };
}


export const secretSchema = z.union([
  z.string(),
  z.strictObject({ value: z.string() }),
  z.strictObject({ env: z.string().min(1) }),
  z.strictObject({ prompt: z.string().min(1) }),
  z.strictObject({ command: z.string().min(1) }),
]);
export type SecretSource = z.infer<typeof secretSchema>;

const secrets = z.record(z.string(), secretSchema);
const positiveInt = z.number().int().positive();

export const configSchema = z.strictObject({
  target: z.strictObject({
    url: z.url().regex(/^wss?:\/\//, 'Expected a ws:// or wss:// URL'),
    headers: secrets.default({}),
    query: secrets.default({}),
  }),
  host: z.strictObject({
    command: z.string().min(1),
    cwd: z.string().default('.'),
    env: z.record(z.string(), z.string()).default({}),
    readyTimeoutMs: positiveInt.default(120_000),
  }).optional(),
  auth: z.strictObject({
    tokens: z.array(z.strictObject({ resource: z.string().min(1), token: secretSchema })).default([]),
  }).prefault({}),
  agent: z.strictObject({
    provider: z.string().min(1).optional(),
    model: z.string().min(1).optional(),
    catalogTimeoutMs: positiveInt.default(30_000),
  }).prefault({}),
  model: z.strictObject({ enabled: z.boolean().default(true), deterministic: z.boolean().default(false) }).prefault({}),
  quirks: z.strictObject({ lazySessionInit: z.boolean().default(false) }).prefault({}),
  workspace: z.strictObject({
    scratchDir: z.string().min(1).optional(),
    sharedFilesystem: z.boolean().default(true),
    keep: z.boolean().default(false),
  }).prefault({}),
  run: z.strictObject({
    concurrency: positiveInt.default(4),
    timeoutMs: positiveInt.default(30_000),
    include: z.array(z.string()).default([]),
    exclude: z.array(z.string()).default([]),
    proposed: z.boolean().default(false),
  }).prefault({}),
  report: z.strictObject({
    reporter: z.enum(['auto', 'cli', 'html']).default('auto'),
    open: z.boolean().default(true),
    htmlFile: z.string().min(1).default('ahp-conformance-report.html'),
    jsonFile: z.string().min(1).optional(),
  }).prefault({}),
});

export type ConfigInput = z.input<typeof configSchema>;
export type ValidatedConfig = z.output<typeof configSchema>;
