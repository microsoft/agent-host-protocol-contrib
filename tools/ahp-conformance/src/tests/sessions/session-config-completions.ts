import type { SessionConfigCompletionsResult } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, resolveConfig } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/session-config-completions/items',
    area: 'Root & sessions',
    feature: 'sessionConfigCompletions',
    title: 'Dynamic session config completions return value-and-label items.',
    spec: '/reference/root#sessionconfigcompletions',
    async run(t) {
      const client = await t.client();
      const config = await resolveConfig(client, t.profile.agent.provider);
      const property = Object.entries(config.schema.properties).find(([, schema]) => schema.enumDynamic === true)?.[0];
      if (property === undefined) t.skip('No dynamically enumerated session configuration property is advertised.');
      const result = await client.request<SessionConfigCompletionsResult>('sessionConfigCompletions', {
        channel: ROOT, provider: t.profile.agent.provider, config: config.values, property,
      });
      assert.ok(Array.isArray(result.items));
      for (const item of result.items) {
        assert.equal(typeof item.value, 'string');
        assert.equal(typeof item.label, 'string');
        if (item.description !== undefined) assert.equal(typeof item.description, 'string');
      }
    },
  },
];
