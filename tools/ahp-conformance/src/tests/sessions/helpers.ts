import type {
  ActionEnvelope,
  CreateSessionParams,
  ListSessionsResult,
  ResolveSessionConfigResult,
  RootState,
  SessionConfigState,
  SessionState,
} from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { Client, LooseStateAction, TestContext } from '../../harness/types.ts';

export const ROOT = 'ahp-root://';

export function rootState(client: Client): RootState {
  const state = client.state<RootState>(ROOT);
  assert.ok(state, 'root subscription must have a state');
  return state;
}

export function sessionState(client: Client, uri: string): SessionState {
  const state = client.state<SessionState>(uri);
  assert.ok(state, 'session subscription must have a state');
  return state;
}

export async function createSession(t: TestContext, client: Client, overrides: Partial<CreateSessionParams> = {}) {
  const uri = `ahp-session:/${t.uniqueId('session')}`;
  const result = await client.request('createSession', {
    channel: uri,
    provider: t.profile.agent.provider,
    ...overrides,
  });
  let disposed = false;
  async function dispose() {
    if (disposed) return;
    const result = await client.request('disposeSession', { channel: uri });
    disposed = true;
    return result;
  }
  t.defer(dispose);
  return { uri, result, dispose };
}

export async function catalogue(client: Client): Promise<ListSessionsResult> {
  const page = await client.request<ListSessionsResult>('listSessions', { channel: ROOT }, { timeoutMs: 5_000 });
  assert.ok(Array.isArray(page.items), 'listSessions must return an items array');
  return page;
}

export async function listedSession(t: TestContext, client: Client, uri: string) {
  const page = await catalogue(client);
  const summary = page.items.find((entry) => entry.resource === uri);
  if (summary === undefined && typeof page.nextCursor === 'string') {
    return t.skip('Created session is outside the initial catalog page; cursor traversal is tested separately.');
  }
  assert.ok(summary, 'created session must be present in the complete initial catalog page');
  return summary;
}

export async function rename(client: Client, uri: string, title: string, prerequisite?: TestContext): Promise<ActionEnvelope> {
  const clientSeq = client.dispatch(uri, { type: 'session/titleChanged', title } satisfies LooseStateAction);
  const envelope = await client.waitForAction(uri, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq,
  );
  if (envelope.rejectionReason !== undefined && prerequisite) {
    return prerequisite.skip('Rename was rejected; sessions/session-actions/title-changed tests rename acceptance.');
  }
  assert.equal(envelope.rejectionReason, undefined, 'a valid rename must be accepted');
  return envelope;
}

export function assertRecord(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

export function configChange(t: TestContext, config: SessionConfigState | undefined, mutable = true) {
  if (!config) t.skip('Session does not expose a configuration schema.');
  for (const [property, schema] of Object.entries(config.schema.properties)) {
    if (Boolean(schema.sessionMutable) !== mutable || schema.readOnly || schema.enumDynamic) continue;
    const current = config.values[property];
    const choices = schema.enum ?? (schema.type === 'boolean' ? [false, true] : []);
    const value = choices.find((candidate) => candidate !== current);
    if (value !== undefined) return { property, value };
  }
  t.skip(`No ${mutable ? 'mutable' : 'immutable'} boolean or enumerated session property has an alternative value.`);
}

export function resolveConfig(client: Client, provider: string) {
  return client.request<ResolveSessionConfigResult>('resolveSessionConfig', { channel: ROOT, provider });
}

export function changeConfig(client: Client, uri: string, property: string, value: unknown) {
  const clientSeq = client.dispatch(uri, { type: 'session/configChanged', config: { [property]: value } } satisfies LooseStateAction);
  return client.waitForAction(uri,
    (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
}
