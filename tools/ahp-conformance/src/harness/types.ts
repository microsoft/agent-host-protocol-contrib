// Shared contracts between the runner, the reporters, and the test areas.
// Test areas code against these interfaces only; the runner implements them.

import type {
  ActionEnvelope,
  AgentInfo,
  CreateSessionParams,
  InitializeResult,
  RootState,
  SessionModelInfo,
  Snapshot,
  StateAction,
} from '@microsoft/agent-host-protocol';
import type { ResolvedConfig } from '../config/schema.ts';

// ─── Test definitions ────────────────────────────────────────────────────────

export const AREAS = [
  'Transport & lifecycle',
  'Root & sessions',
  'Chats & turns',
  'Resources',
  'Tools & permissions',
  'Authentication',
  'Terminals',
  'Automations',
  'Telemetry',
  /** Proposed, not-yet-ratified behavior; every test here requires `requires.proposed`. */
  'Proposed',
] as const;
export type Area = (typeof AREAS)[number];

export interface TestDefinition {
  /** Stable, unique, path-like id: `<area-slug>/<feature>/<case>`, e.g. `resources/read/not-found`. */
  id: string;
  area: Area;
  /** Row group within the area, usually the method or feature name, e.g. `resourceRead`. */
  feature: string;
  /** One-sentence description of the single behavior under test. */
  title: string;
  /** Spec anchor relative to https://microsoft.github.io/agent-host-protocol, e.g. `/reference/commands#resourceread`. */
  spec?: string;
  /** All requirements must hold or the test is reported as skipped with the failing requirement's reason. */
  requires?: Requirement[];
  /** Overrides `run.timeoutMs` from the config. */
  timeoutMs?: number;
  run(t: TestContext): Promise<void>;
}

/** Facts gathered once, before any test runs, from a probe connection. */
export interface HostProfile {
  initialize: InitializeResult;
  root: RootState;
  /** The agent selected by `agent.provider` (or the first advertised agent). */
  agent: AgentInfo;
  /** The model selected by `agent.model` (or the agent's first model), if any. */
  model?: SessionModelInfo;
  config: ResolvedConfig;
}

export interface Requirement {
  /** Short label shown in reports, e.g. `multipleChats.fork`. */
  label: string;
  /** `capability`: host did not advertise it. `config`: disabled or unconfigured by the user. */
  kind: 'capability' | 'config';
  check(profile: HostProfile): boolean;
}

// ─── Test context ────────────────────────────────────────────────────────────

export interface TestContext {
  readonly id: string;
  readonly profile: HostProfile;
  readonly config: ResolvedConfig;
  /** Aborted on timeout or run cancellation. */
  readonly signal: AbortSignal;
  /** Absolute local path of an empty directory owned by this test, inside the configured scratch dir. */
  readonly scratchDir: string;
  /** `file:` URI for `scratchDir` joined with an optional relative path. */
  scratchUri(relativePath?: string): string;
  log(message: string, data?: unknown): void;
  /** Unique, readable id for clientIds, phrases, file names, etc. */
  uniqueId(prefix?: string): string;
  /** Registers cleanup that runs (in reverse order) after the test, pass or fail. */
  defer(cleanup: () => unknown): void;
  /** Ends the test as skipped, e.g. when a precondition can only be discovered at runtime. */
  skip(reason: string): never;
  /** Opens a raw, uninitialized connection with transport auth applied. Closed automatically after the test. */
  connect(options?: { label?: string }): Promise<Connection>;
  /** `connect()` + `initialize` + `authenticate` for each configured token. */
  client(options?: ClientOptions): Promise<Client>;
  /**
   * Creates a session for the selected agent/model whose working directory is `scratchDir`,
   * subscribes to it and its default chat, and waits for it to become ready. Disposed after the test.
   */
  session(client: Client, overrides?: Partial<CreateSessionParams>): Promise<SessionHandle>;
}

export interface ClientOptions {
  label?: string;
  clientId?: string;
  /** Defaults to `['ahp-root://']`. */
  initialSubscriptions?: string[];
  /** Defaults to `[PROTOCOL_VERSION]`. */
  protocolVersions?: string[];
  /** Push configured auth tokens after initialize. Defaults to `true`. */
  authenticate?: boolean;
}

// ─── Wire access ─────────────────────────────────────────────────────────────

export type JsonRpcMessage = Record<string, unknown> & { jsonrpc: '2.0' };

/**
 * `StateAction` with plain string literal `type`s. The SDK's `ActionType` is an ambient
 * const enum that erasable TypeScript cannot reference, so tests write `type: 'chat/turnStarted'`.
 */
export type LooseStateAction = StateAction extends infer A
  ? A extends { type: infer T extends string }
    ? Omit<A, 'type'> & { type: `${T}` }
    : never
  : never;

export class RpcError extends Error {
  readonly code: number;
  readonly data: unknown;
  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.name = 'RpcError';
    this.code = code;
    this.data = data;
  }
}

export interface ReceivedFrame {
  /** Raw frame text exactly as received. */
  text: string;
  /** Size of `text` as UTF-16 code units × 2. */
  utf16Bytes: number;
  /** The parsed message, identical (===) to its entry in `received`; absent for non-object JSON or invalid text. */
  message?: JsonRpcMessage;
}

export interface WaitOptions {
  /** Defaults to the test's remaining time. */
  timeoutMs?: number;
}

/** A raw JSON-RPC connection. Every frame is recorded in the test transcript. */
export interface Connection {
  readonly label: string;
  /** Sends a request and resolves with `result`, or rejects with `RpcError`. */
  request<R = unknown>(method: string, params?: unknown, options?: WaitOptions): Promise<R>;
  notify(method: string, params?: unknown): void;
  /** Sends a text frame verbatim (for malformed-JSON tests). */
  sendRaw(text: string): void;
  /** Resolves with the first received message (already buffered or future) matching `predicate`. */
  waitForMessage(predicate: (message: JsonRpcMessage) => boolean, options?: WaitOptions): Promise<JsonRpcMessage>;
  /** Resolves with the params of the first matching notification (already buffered or future). */
  waitForNotification<P = unknown>(method: string, predicate?: (params: P) => boolean, options?: WaitOptions): Promise<P>;
  /** Fails if a matching notification arrives within `durationMs`. */
  expectNoNotification<P = unknown>(method: string, predicate: (params: P) => boolean, durationMs: number): Promise<void>;
  /** Handles host → client requests (e.g. `resourceRead`). Unhandled methods get `-32601`. */
  onRequest(method: string, handler: (params: unknown) => unknown | Promise<unknown>): void;
  /** Every message received so far, in order. */
  readonly received: readonly JsonRpcMessage[];
  /** Every text frame received so far, in order, including frames that aren't JSON-RPC objects. */
  readonly receivedFrames: readonly ReceivedFrame[];
  /** Resolves when the socket closes. */
  readonly closed: Promise<{ code: number; reason: string }>;
  close(): Promise<void>;
}

/**
 * An initialized connection that mirrors subscribed channel state by applying
 * received action envelopes with the SDK reducers.
 */
export interface Client extends Connection {
  readonly clientId: string;
  readonly initializeResult: InitializeResult;
  /** Highest `serverSeq` observed on this connection. */
  readonly lastServerSeq: number;
  subscribe(uri: string): Promise<Snapshot>;
  unsubscribe(uri: string): Promise<void>;
  /** Reducer-applied mirror of a subscribed channel's state. */
  state<S = unknown>(uri: string): S | undefined;
  /** Sends `dispatchAction` with the next `clientSeq` and returns that `clientSeq`. */
  dispatch(channel: string, action: LooseStateAction): number;
  /** Action envelopes received so far, optionally filtered by channel. */
  actions(channel?: string): readonly ActionEnvelope[];
  waitForAction(channel: string, predicate: (envelope: ActionEnvelope) => boolean, options?: WaitOptions): Promise<ActionEnvelope>;
  /** Resolves once the mirrored state of `uri` satisfies `predicate` (checked immediately and after every action). */
  waitForState<S = unknown>(uri: string, predicate: (state: S) => boolean, options?: WaitOptions): Promise<S>;
  /**
   * Asserts that this client's reducer-applied mirror of `uri` deep-equals a fresh
   * snapshot taken from a new connection.
   */
  assertMirrorMatchesSnapshot(uri: string): Promise<void>;
}

export interface SessionHandle {
  readonly sessionUri: string;
  /** The session's default chat channel URI. */
  readonly chatUri: string;
  /**
   * Dispatches `chat/turnStarted` with a user message on `chatUri` (or `options.chatUri`)
   * and returns the new turn id.
   */
  startTurn(text: string, options?: { chatUri?: string }): string;
  /**
   * Waits until the turn is no longer active and returns its final state as recorded
   * in the chat mirror's `turns`.
   */
  waitForTurnEnd<T = unknown>(turnId: string, options?: WaitOptions & { chatUri?: string }): Promise<T>;
  /** Concatenated markdown text of the turn's assistant response parts. */
  responseText(turnId: string, options?: { chatUri?: string }): string;
  /**
   * Resolves once the session is `ready`. With `quirks.lazySessionInit`, first runs one short
   * model turn on the default chat to trigger the host's deferred initialization.
   */
  ready(): Promise<void>;
  dispose(): Promise<void>;
}

// ─── Run events (runner → reporters) ─────────────────────────────────────────

export type TestStatus = 'queued' | 'running' | 'passed' | 'failed' | 'skipped';

export interface TestSummary {
  id: string;
  area: Area;
  feature: string;
  title: string;
  spec?: string;
  requires: string[];
}

export interface FrameRecord {
  connection: string;
  direction: 'send' | 'recv';
  /** Parsed JSON when possible, otherwise the raw text. Secrets are redacted. */
  message: unknown;
}

export interface ProfileSummary {
  /** Endpoint with userinfo and query values redacted. */
  endpoint: string;
  protocolVersion: string;
  serverInfo?: { name: string; version?: string; title?: string };
  agent: { provider: string; displayName: string; capabilities?: unknown };
  model?: string;
  scratchDir: string;
}

export type RunEvent =
  | { type: 'run-started'; at: string; tests: TestSummary[] }
  | { type: 'profile'; at: string; profile: ProfileSummary }
  | { type: 'test-started'; at: string; id: string }
  | { type: 'test-log'; at: string; id: string; message: string; data?: unknown }
  | { type: 'test-frame'; at: string; id: string; frame: FrameRecord }
  | {
      type: 'test-finished';
      at: string;
      id: string;
      status: 'passed' | 'failed' | 'skipped';
      durationMs: number;
      /** Skip reason or failure message. */
      reason?: string;
      error?: { message: string; stack?: string; expected?: unknown; actual?: unknown };
    }
  | { type: 'run-error'; at: string; message: string }
  | { type: 'run-finished'; at: string; passed: number; failed: number; skipped: number; durationMs: number };

export interface Reporter {
  onEvent(event: RunEvent): void;
  /** Called once after `run-finished` (or a fatal error); resolves when output is flushed. */
  close(): Promise<void>;
}
