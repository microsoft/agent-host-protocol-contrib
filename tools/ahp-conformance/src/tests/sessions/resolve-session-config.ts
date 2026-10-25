import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { assertRecord, resolveConfig } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/resolve-session-config/schema',
    area: 'Root & sessions',
    feature: 'resolveSessionConfig',
    title: 'Resolving session config returns an object schema with property descriptors.',
    spec: '/reference/session#sessionconfigschema',
    async run(t) {
      const client = await t.client();
      const result = await resolveConfig(client, t.profile.agent.provider);
      assert.equal(result.schema.type, 'object');
      assertRecord(result.schema.properties);
      for (const property of Object.values(result.schema.properties)) {
        assert.equal(typeof property.title, 'string');
        assert.ok(['string', 'number', 'boolean', 'array', 'object'].includes(property.type));
      }
      if (result.schema.required !== undefined) {
        assert.ok(Array.isArray(result.schema.required));
        for (const property of result.schema.required) assert.equal(typeof property, 'string');
      }
    },
  },
  {
    id: 'sessions/resolve-session-config/values',
    area: 'Root & sessions',
    feature: 'resolveSessionConfig',
    title: 'Resolving session config returns a configuration-values object.',
    spec: '/reference/root#resolvesessionconfig',
    async run(t) {
      const client = await t.client();
      const result = await resolveConfig(client, t.profile.agent.provider);
      assertRecord(result.values);
    },
  },
];
