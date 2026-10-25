import { AhpErrorCodes, MessageKind, automationReducer } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, AutomationDefinition, AutomationEntry, AutomationRunState, AutomationState, RootState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, LooseStateAction, TestContext } from '../../harness/types.ts';

export const CATALOGUE = 'ahp-automations://';
const CLEANUP_WAIT = 1200;

export function record(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

function uri(value: unknown, scheme: string): asserts value is string {
  assert.equal(typeof value, 'string');
  assert.equal(new URL(value as string).protocol, scheme);
}

export function assertEntry(value: unknown): asserts value is AutomationEntry {
  record(value);
  uri(value.resource, 'ahp-automation:');
  record(value.definition);
  assert.equal(typeof value.definition.title, 'string');
  assert.equal(typeof value.definition.enabled, 'boolean');
  assert.ok(Array.isArray(value.definition.triggers));
  record(value.definition.session);
  record(value.definition.message);
  assert.equal(typeof value.definition.message.text, 'string');
  record(value.definition.message.origin);
  assert.equal(value.definition.message.origin.kind, 'automation');
  assert.ok(Array.isArray(value.runs));
  assert.ok(Array.isArray(value.operations));
  for (const operation of value.operations) assert.equal(typeof operation, 'string');
  for (const field of ['createdAt', 'modifiedAt']) {
    assert.equal(typeof value[field], 'string');
    assert.ok(Number.isFinite(Date.parse(value[field] as string)), `invalid ${field}`);
  }
}

export interface CatalogueSnapshot {
  fromSeq: number;
  state: AutomationState;
}

export async function catalogue(client: Client, timeoutMs?: number): Promise<CatalogueSnapshot> {
  const result = await client.request('subscribe', { channel: CATALOGUE }, { timeoutMs });
  record(result);
  record(result.snapshot);
  assert.equal(result.snapshot.resource, CATALOGUE);
  assert.equal(typeof result.snapshot.fromSeq, 'number');
  record(result.snapshot.state);
  assert.ok(Array.isArray(result.snapshot.state.entries));
  for (const entry of result.snapshot.state.entries) {
    record(entry);
    uri(entry.resource, 'ahp-automation:');
  }
  const resources = result.snapshot.state.entries.map((entry) => entry.resource);
  assert.equal(resources.length, new Set(resources).size, 'catalogue resource keys must be unique');
  return { fromSeq: result.snapshot.fromSeq as number, state: result.snapshot.state as unknown as AutomationState };
}

export function afterActions(client: Client, before: CatalogueSnapshot, through: number): AutomationState {
  return client.actions(CATALOGUE).reduce((state, envelope) => {
    if (envelope.serverSeq <= before.fromSeq || envelope.serverSeq > through || envelope.rejectionReason !== undefined) return state;
    return automationReducer(state, envelope.action as Parameters<typeof automationReducer>[1]);
  }, before.state);
}

type CatalogueRequest = Extract<LooseStateAction, {
  type: 'automation/createRequested' | 'automation/updateRequested' | 'automation/removed';
}>;

export async function dispatchOutcome(client: Client, action: CatalogueRequest, timeoutMs = 5000): Promise<ActionEnvelope> {
  const fromSeq = client.lastServerSeq;
  const clientSeq = client.dispatch(CATALOGUE, action);
  return client.waitForAction(CATALOGUE, (envelope) => {
    if (envelope.serverSeq <= fromSeq) return false;
    if (envelope.origin?.clientId === client.clientId && envelope.origin.clientSeq === clientSeq) return true;
    if (envelope.action.type === action.type && 'resource' in envelope.action && envelope.action.resource === action.resource) return true;
    // Successful side-effect requests publish a server-originated full entry.
    return action.type !== 'automation/removed' && envelope.action.type === 'automation/set' &&
      envelope.action.automation.resource === action.resource;
  }, { timeoutMs });
}

export async function accepted(client: Client, action: CatalogueRequest): Promise<ActionEnvelope> {
  const envelope = await dispatchOutcome(client, action);
  assert.equal(envelope.rejectionReason, undefined, envelope.rejectionReason ?? 'automation action was rejected');
  return envelope;
}

export function runtimeDisabled(client: Client): boolean {
  const config = client.state<RootState>('ahp-root://')?.config;
  const property = config?.schema.properties.automationsEnabled;
  return property?.type === 'boolean' && (config?.values.automationsEnabled ?? property.default) === false;
}

export async function setAfter(client: Client, resource: string, fromSeq: number): Promise<AutomationEntry> {
  const envelope = await client.waitForAction(CATALOGUE, (event) => event.serverSeq > fromSeq &&
    event.action.type === 'automation/set' && event.action.automation.resource === resource);
  assert.equal(envelope.rejectionReason, undefined);
  assert.equal(envelope.action.type, 'automation/set');
  const entry = (envelope.action as { automation: unknown }).automation;
  record(entry);
  assert.equal(entry.resource, resource);
  return entry as unknown as AutomationEntry;
}

export function manualDefinition(t: TestContext): AutomationDefinition {
  return {
    title: t.uniqueId('ahp-conformance-manual'),
    message: { text: prompts.reply('ok'), origin: { kind: MessageKind.Automation } },
    session: {
      provider: t.profile.agent.provider,
      ...(t.profile.model && { model: { id: t.profile.model.id } }),
    },
    enabled: false,
    triggers: [],
  };
}

function terminal(state: AutomationRunState): boolean {
  return ['completed', 'failed', 'cancelled'].includes(state.lifecycle.status);
}

async function runSnapshot(client: Client, resource: string): Promise<AutomationRunState> {
  const result = await client.request('subscribe', { channel: resource }, { timeoutMs: CLEANUP_WAIT });
  record(result);
  record(result.snapshot);
  record(result.snapshot.state);
  return result.snapshot.state as unknown as AutomationRunState;
}

async function stopRun(client: Client, resource: string): Promise<void> {
  let state = await runSnapshot(client, resource);
  try {
    if (!terminal(state)) {
      assert.ok(client.initializeResult.automations?.runCancellation !== undefined, 'cannot safely cancel an automation run');
      client.dispatch(resource, { type: 'automationRun/cancelRequested' });
      state = await runSnapshot(client, resource);
      if (!terminal(state)) {
        await client.waitForAction(resource, (event) => event.action.type === 'automationRun/lifecycleChanged' &&
          ['completed', 'failed', 'cancelled'].includes(event.action.lifecycle.status), { timeoutMs: CLEANUP_WAIT });
        state = await runSnapshot(client, resource);
      }
      assert.ok(terminal(state), 'automation run did not stop during cleanup');
    }
  } finally {
    const sessions = new Set(state.sessions);
    for (const event of client.actions(resource)) {
      if (event.action.type === 'automationRun/sessionSet' && event.rejectionReason === undefined) sessions.add(event.action.session);
    }
    await Promise.all([...sessions].map(async (channel) => {
      try { await client.request('disposeSession', { channel }, { timeoutMs: CLEANUP_WAIT }); }
      catch (error) {
        if (!(error instanceof RpcError && (error.code === AhpErrorCodes.SessionNotFound || error.code === AhpErrorCodes.NotFound))) throw error;
      }
    }));
  }
}

export interface OwnedAutomation {
  resource: string;
  runs: Set<string>;
  mayHaveRuns: boolean;
  cleanup(): Promise<void>;
}

export function ownAutomation(t: TestContext, client: Client): OwnedAutomation {
  let cleaned = false;
  const owned: OwnedAutomation = {
    resource: `ahp-automation:/${t.uniqueId('manual')}`,
    runs: new Set(),
    mayHaveRuns: false,
    async cleanup() {
      if (cleaned) return;
      const errors: unknown[] = [];
      if (owned.mayHaveRuns) {
        try {
          const entry = (await catalogue(client, CLEANUP_WAIT)).state.entries.find((value) => value.resource === owned.resource);
          for (const run of entry?.runs ?? []) owned.runs.add(run.resource);
        } catch (error) { errors.push(error); }
        const stopped = await Promise.allSettled([...owned.runs].map((resource) => stopRun(client, resource)));
        for (const result of stopped) if (result.status === 'rejected') errors.push(result.reason);
      }
      try {
        const present = (await catalogue(client, CLEANUP_WAIT)).state.entries.some((entry) => entry.resource === owned.resource);
        if (present) {
          const removed = await dispatchOutcome(client, { type: 'automation/removed', resource: owned.resource }, CLEANUP_WAIT);
          assert.equal(removed.rejectionReason, undefined, 'cleanup removal was rejected');
        } else {
          client.dispatch(CATALOGUE, { type: 'automation/removed', resource: owned.resource });
        }
        assert.ok(!(await catalogue(client, CLEANUP_WAIT)).state.entries.some((entry) => entry.resource === owned.resource), 'automation survived cleanup');
      } catch (error) { errors.push(error); }
      if (errors.length > 0) throw new AggregateError(errors, `Cleaning up ${owned.resource}`);
      cleaned = true;
    },
  };
  t.defer(owned.cleanup);
  return owned;
}

export async function createAutomation(t: TestContext, client: Client, options: { enabled?: boolean } = {}) {
  const owned = ownAutomation(t, client);
  const definition = { ...manualDefinition(t), enabled: options.enabled ?? false };
  const fromSeq = client.lastServerSeq;
  await accepted(client, { type: 'automation/createRequested', resource: owned.resource, definition });
  const entry = await setAfter(client, owned.resource, fromSeq);
  assert.ok(entry.operations.includes('remove' as AutomationEntry['operations'][number]), 'test-created automations must be removable for cleanup');
  return Object.assign(owned, { definition, entry });
}

export async function requireOperation(t: TestContext, client: Client, resource: string, operation: 'update' | 'run'): Promise<AutomationEntry> {
  const entry = (await catalogue(client)).state.entries.find((value) => value.resource === resource);
  assert.ok(entry, 'test-owned automation is missing');
  if (!entry.operations.includes(operation as AutomationEntry['operations'][number])) {
    t.skip(`The test-created automation does not currently permit ${operation}.`);
  }
  return entry;
}
