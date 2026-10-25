import type { SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, jsonValue } from './coverage-helpers.ts';
import { changeConfig, configChange, sessionState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/session-config/mutable-value',
    area: 'Root & sessions',
    feature: 'sessionConfig',
    title: 'A client can change a session-mutable configuration value.',
    spec: '/reference/session#sessionconfigchanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const choice = configChange(t, sessionState(client, session.sessionUri).config);
      const echo = await changeConfig(client, session.sessionUri, choice.property, choice.value);
      assert.equal(echo.rejectionReason, undefined);
      assert.deepEqual(jsonValue(sessionState(client, session.sessionUri).config?.values[choice.property]), jsonValue(choice.value));
    },
  },
  {
    id: 'sessions/session-config/merge-preserves-values',
    area: 'Root & sessions',
    feature: 'sessionConfig',
    title: 'A session config update preserves other configuration values by default.',
    spec: '/reference/session#sessionconfigchanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const config = sessionState(client, session.sessionUri).config;
      const choice = configChange(t, config);
      const unchanged = Object.entries(config!.values).filter(([property]) => property !== choice.property);
      if (unchanged.length === 0) t.skip('No other resolved configuration value is available to check merge behavior.');
      const echo = await changeConfig(client, session.sessionUri, choice.property, choice.value);
      assert.equal(echo.rejectionReason, undefined);
      const after = sessionState(client, session.sessionUri).config?.values;
      for (const [property, value] of unchanged) assert.deepEqual(jsonValue(after?.[property]), jsonValue(value));
    },
  },
  {
    id: 'sessions/session-config/immutable-value-rejected',
    area: 'Root & sessions',
    feature: 'sessionConfig',
    title: 'A client cannot change a configuration property that is not session-mutable.',
    spec: '/reference/session#sessionconfigchanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const choice = configChange(t, sessionState(client, session.sessionUri).config, false);
      const echo = await changeConfig(client, session.sessionUri, choice.property, choice.value);
      assert.notEqual(echo.rejectionReason, undefined);
    },
  },
  {
    id: 'sessions/session-config/mirror-consistency',
    area: 'Root & sessions',
    feature: 'sessionConfig',
    title: 'The session mirror matches a fresh snapshot after a configuration change.',
    spec: '/reference/session#sessionconfigchanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const choice = configChange(t, sessionState(client, session.sessionUri).config);
      const echo = await changeConfig(client, session.sessionUri, choice.property, choice.value);
      assert.equal(echo.rejectionReason, undefined);
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
  {
    id: 'sessions/session-config/replace-drops-omitted-values',
    area: 'Root & sessions', feature: 'sessionConfig',
    title: 'Replacing session configuration drops omitted optional values instead of merging them.',
    spec: '/reference/session#sessionconfigchangedaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const config = sessionState(client, session.sessionUri).config;
      if (!config) return t.skip('The session does not expose a configuration schema.');
      const keys = Object.keys(config.values);
      if (keys.length < 2) t.skip('Fewer than two resolved values are available for a nonempty replacement.');
      if (keys.some((key) => !config.schema.properties[key]?.sessionMutable || config.schema.properties[key].readOnly)) {
        t.skip('Existing immutable/read-only values prevent a safe full replacement in this schema.');
      }
      const omit = keys.find((key) => !config.schema.required?.includes(key));
      if (!omit) return t.skip('No optional resolved property can be omitted from a valid replacement.');
      const replacement = Object.fromEntries(Object.entries(config.values).filter(([key]) => key !== omit));
      await accepted(client, session.sessionUri, { type: 'session/configChanged', config: replacement, replace: true });
      const observer = await t.client({ initialSubscriptions: [] });
      const fresh = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.deepEqual(jsonValue(fresh.config?.values), jsonValue(replacement));
      assert.ok(!Object.hasOwn(fresh.config?.values ?? {}, omit));
    },
  },
];
