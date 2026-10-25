import type { RootState, Snapshot, SubscribeResult } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { Client, TestContext, TestDefinition } from '../../harness/types.ts';
import { ROOT, rootState } from './helpers.ts';

let configQueue: Promise<void> = Promise.resolve();

async function lockConfig(t: TestContext): Promise<void> {
  const previous = configQueue;
  let release!: () => void;
  const held = new Promise<void>((resolve) => { release = resolve; });
  configQueue = previous.then(() => held);
  t.defer(release);
  let removeAbort = () => {};
  try {
    t.signal.throwIfAborted();
    await Promise.race([previous, new Promise<never>((_resolve, reject) => {
      const abort = () => reject(t.signal.reason);
      t.signal.addEventListener('abort', abort, { once: true });
      removeAbort = () => t.signal.removeEventListener('abort', abort);
      if (t.signal.aborted) abort();
    })]);
    t.signal.throwIfAborted();
  } finally {
    removeAbort();
  }
}

async function changeConfig(client: Client, config: Record<string, unknown>, replace = false) {
  const clientSeq = client.dispatch(ROOT, { type: 'root/configChanged', config, ...(replace && { replace }) });
  const echo = await client.waitForAction(ROOT, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
  assert.equal(echo.rejectionReason, undefined, 'a valid mutable root configuration change must be accepted');
  assert.equal(echo.action.type, 'root/configChanged');
  return echo;
}

async function mutableConfig(t: TestContext, removable = false) {
  await lockConfig(t);
  const client = await t.client();
  const config = rootState(client).config;
  if (config === undefined) t.skip('The root snapshot does not advertise host configuration.');
  // Limit global mutations to reversible presentation/logging choices, never auth, policy, or execution settings.
  const safeNames = new Set(['loglevel', 'logginglevel', 'verbosity', 'verbose', 'theme', 'colortheme', 'wordwrap', 'showtimestamps']);
  for (const [property, schema] of Object.entries(config.schema.properties)) {
    const name = property.split(/[./]/).at(-1)!.replace(/[^a-z]/gi, '').toLowerCase();
    if (!safeNames.has(name) || schema.readOnly || !Object.hasOwn(config.values, property)) continue;
    if (removable && config.schema.required?.includes(property)) continue;
    const current = config.values[property];
    const choices = schema.enum ?? (schema.type === 'boolean' ? [false, true] : []);
    const value = choices.find((candidate) => candidate !== null && typeof candidate === typeof current && candidate !== current);
    if (value === undefined) continue;
    const original = structuredClone(config.values);
    let changed = false;
    t.defer(async () => {
      if (changed) await changeConfig(client, original, true);
    });
    return {
      client, property, value, original,
      async change(values: Record<string, unknown>, replace = false) {
        changed = true;
        return changeConfig(client, values, replace);
      },
    };
  }
  t.skip(`No ${removable ? 'optional ' : ''}mutable presentation/logging root property advertises a safe boolean or enum alternative.`);
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/root-config/echo-to-subscribers',
    area: 'Root & sessions', feature: 'rootConfig',
    title: 'A mutable root config dispatch is echoed to every root subscriber with its origin.',
    spec: '/reference/root#rootconfigchangedaction',
    async run(t) {
      const config = await mutableConfig(t);
      const observer = await t.client();
      const echo = await config.change({ [config.property]: config.value });
      const broadcast = await observer.waitForAction(ROOT, (event) =>
        event.origin?.clientId === config.client.clientId && event.origin.clientSeq === echo.origin?.clientSeq);
      assert.equal(broadcast.rejectionReason, undefined);
      assert.deepEqual(broadcast, echo);
    },
  },
  {
    id: 'sessions/root-config/merge-preserves-values',
    area: 'Root & sessions', feature: 'rootConfig',
    title: 'A partial root config change preserves other resolved values by default.',
    spec: '/reference/root#rootconfigchangedaction',
    async run(t) {
      const config = await mutableConfig(t);
      const others = Object.entries(config.original).filter(([key]) => key !== config.property);
      if (others.length === 0) t.skip('Root configuration has no second resolved value to check merge preservation.');
      await config.change({ [config.property]: config.value });
      const values = rootState(config.client).config!.values;
      assert.deepEqual(values[config.property], config.value);
      for (const [key, value] of others) assert.deepEqual(values[key], value, `${key} must survive a merge`);
    },
  },
  {
    id: 'sessions/root-config/replace-removes-omitted-value',
    area: 'Root & sessions', feature: 'rootConfig',
    title: 'Replacing root config values removes an omitted optional value.',
    spec: '/reference/root#rootconfigchangedaction',
    async run(t) {
      const config = await mutableConfig(t, true);
      await config.change({ [config.property]: config.value });
      const replacement = { ...config.original };
      delete replacement[config.property];
      await config.change(replacement, true);
      assert.equal(Object.hasOwn(rootState(config.client).config!.values, config.property), false);
    },
  },
  {
    id: 'sessions/root-config/persisted-state-matches-echo',
    area: 'Root & sessions', feature: 'rootConfig',
    title: 'A fresh root snapshot agrees with reducer-mirrored config values after a change.',
    spec: '/reference/root#rootconfigchangedaction',
    async run(t) {
      const config = await mutableConfig(t);
      await config.change({ [config.property]: config.value });
      const expected = rootState(config.client).config!.values;
      assert.deepEqual(expected[config.property], config.value);
      const reader = await t.client({ initialSubscriptions: [] });
      const result = await reader.request<SubscribeResult>('subscribe', { channel: ROOT });
      const snapshot: Snapshot | undefined = result.snapshot;
      assert.ok(snapshot, 'a root subscription must return a snapshot');
      assert.deepEqual((snapshot.state as RootState).config?.values, expected);
    },
  },
];
