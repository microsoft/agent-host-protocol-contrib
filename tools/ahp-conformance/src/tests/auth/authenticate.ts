import type { RootState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { authenticate, authenticateAll, configuredToken, observeAuthRequired, protectedResources, requiredTokens, revoke } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'auth/authenticate/invalid-advertised-token-rejected',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'authenticate rejects an invalid nonempty token for an advertised resource.',
    spec: '/specification/authentication#token-delivery',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const [resource] = protectedResources(t, client);
      assert.ok(resource);
      await assert.rejects(client.request('authenticate', {
        channel: 'ahp-root://', resource: resource.resource, token: t.uniqueId('invalid-token'),
      }), RpcError);
    },
  },
  {
    id: 'auth/authenticate/unadvertised-resource-rejected',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'authenticate rejects a resource the host has never advertised.',
    spec: '/specification/authentication#token-delivery',
    async run(t) {
      const client = await t.client({ authenticate: false });
      const resource = `https://ahp-conformance.invalid/${encodeURIComponent(t.uniqueId('resource'))}`;
      const root = client.state<RootState>('ahp-root://');
      assert.ok(root, 'expected root state');
      assert.ok(!root.agents.some((agent) => agent.protectedResources?.some((entry) => entry.resource === resource)));
      await assert.rejects(client.request('authenticate', {
        channel: 'ahp-root://', resource, token: t.uniqueId('unadvertised-token'),
      }), RpcError);
    },
  },
  {
    id: 'auth/authenticate/advertised-resource-token-accepted',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'authenticate accepts a configured token for an advertised resource.',
    spec: '/reference/common#authenticate',
    requires: [requires.protectedResources, requires.authTokens],
    async run(t) {
      const client = await t.client({ authenticate: false });
      assert.deepEqual(await authenticate(client, configuredToken(t, client)), {});
    },
  },
  {
    id: 'auth/authenticate/repeated-token-accepted',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'authenticate accepts a repeated valid token without reinitializing the connection.',
    spec: '/specification/authentication#token-delivery',
    requires: [requires.protectedResources, requires.authTokens],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const token = configuredToken(t, client);
      await authenticate(client, token);
      assert.deepEqual(await authenticate(client, token), {});
    },
  },
  {
    id: 'auth/authenticate/empty-token-revocation-accepted',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'authenticate accepts an empty token to revoke an authenticated resource.',
    spec: '/specification/authentication#token-delivery',
    requires: [requires.protectedResources, requires.authTokens],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const token = configuredToken(t, client);
      await authenticate(client, token);
      assert.deepEqual(await revoke(client, token.resource), {});
    },
  },
  {
    id: 'auth/authenticate/revocation-blocks-required-operation',
    area: 'Authentication',
    feature: 'authenticate',
    title: 'Revoking a required resource blocks a previously authenticated session-creation operation.',
    spec: '/specification/authentication#token-delivery',
    requires: [requires.protectedResources, requires.authTokens],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const [token, ...others] = requiredTokens(t, client);
      assert.ok(token);
      await authenticateAll(client, others);
      await observeAuthRequired(t, client);
      await authenticate(client, token);
      await t.session(client);
      await revoke(client, token.resource);
      await assert.rejects(t.session(client), RpcError);
    },
  },
];
