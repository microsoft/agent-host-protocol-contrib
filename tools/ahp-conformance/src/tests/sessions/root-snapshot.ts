import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { rootState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/root-snapshot/agents-array',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'The root snapshot contains an agents array.',
    spec: '/reference/root#rootstate',
    async run(t) {
      const client = await t.client();
      assert.ok(Array.isArray(rootState(client).agents));
    },
  },
  {
    id: 'sessions/root-snapshot/agent-fields',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'Each advertised agent contains the required identity fields.',
    spec: '/reference/root#agentinfo',
    async run(t) {
      const client = await t.client();
      for (const agent of rootState(client).agents) {
        assert.equal(typeof agent.provider, 'string');
        assert.equal(typeof agent.displayName, 'string');
        assert.equal(typeof agent.description, 'string');
      }
    },
  },
  {
    id: 'sessions/root-snapshot/models-array',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'Each advertised agent contains a models array.',
    spec: '/reference/root#agentinfo',
    async run(t) {
      const client = await t.client();
      for (const agent of rootState(client).agents) assert.ok(Array.isArray(agent.models));
    },
  },
  {
    id: 'sessions/root-snapshot/model-fields',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'Each advertised model contains its required identity fields.',
    spec: '/reference/root#sessionmodelinfo',
    async run(t) {
      const client = await t.client();
      for (const agent of rootState(client).agents) {
        for (const model of agent.models) {
          assert.equal(typeof model.id, 'string');
          assert.equal(typeof model.name, 'string');
          assert.equal(typeof model.provider, 'string');
        }
      }
    },
  },
  {
    id: 'sessions/root-snapshot/model-provider',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'Each model names the provider of its advertising agent.',
    spec: '/reference/root#sessionmodelinfo',
    async run(t) {
      const client = await t.client();
      for (const agent of rootState(client).agents) {
        for (const model of agent.models) assert.equal(model.provider, agent.provider);
      }
    },
  },
  {
    id: 'sessions/root-snapshot/active-session-count',
    area: 'Root & sessions',
    feature: 'rootSnapshot',
    title: 'An advertised active-session count is a non-negative integer.',
    spec: '/reference/root#rootstate',
    async run(t) {
      const client = await t.client();
      const count = rootState(client).activeSessions;
      if (count === undefined) return t.skip('Root state does not advertise activeSessions.');
      assert.ok(Number.isInteger(count) && count >= 0);
    },
  },
];
