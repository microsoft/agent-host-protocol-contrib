import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { assertUrl, metadataRules, protectedResources } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'auth/protected-resources/resource-https-url',
    area: 'Authentication',
    feature: 'protectedResources',
    title: 'Protected resource identifiers are absolute HTTPS URLs.',
    spec: '/reference/common#protectedresourcemetadata',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      for (const resource of protectedResources(t, client)) {
        assert.equal(assertUrl(resource.resource).protocol, 'https:');
      }
    },
  },
  {
    id: 'auth/protected-resources/resource-no-fragment',
    area: 'Authentication',
    feature: 'protectedResources',
    title: 'Protected resource identifiers have no fragment component.',
    spec: '/reference/common#protectedresourcemetadata',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      for (const resource of protectedResources(t, client)) {
        assert.equal(typeof resource.resource, 'string');
        assert.ok(!resource.resource.includes('#'), 'resource identifiers must not contain a fragment');
      }
    },
  },
  ...metadataRules.map((rule): TestDefinition => ({
    id: `auth/protected-resources/${rule.id}`,
    area: 'Authentication',
    feature: 'protectedResources',
    title: rule.title,
    spec: '/reference/common#protectedresourcemetadata',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const entries = protectedResources(t, client).filter((entry) => entry[rule.field] !== undefined);
      if (entries.length === 0) t.skip(`The selected agent does not advertise ${rule.field}.`);
      for (const entry of entries) rule.check(entry[rule.field]);
    },
  })),
];
