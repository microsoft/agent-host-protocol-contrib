import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { Requirement, TestDefinition } from '../../harness/types.ts';
import {
  assertRecord, assertResourceMetadata, authenticateAll, authRequiredResources,
  observeAuthRequired, protectedResources, requiredResources, requiredTokens, selectedAgent,
} from './helpers.ts';

const noProtectedResources: Requirement = {
  label: 'agent.noProtectedResources',
  kind: 'capability',
  check: (profile) => (profile.agent.protectedResources?.length ?? 0) === 0,
};

const onlyOptionalResources: Requirement = {
  label: 'agent.onlyOptionalProtectedResources',
  kind: 'capability',
  check: (profile) => profile.agent.protectedResources?.every((resource) => resource.required === false) === true,
};

export const tests: TestDefinition[] = [
  {
    id: 'auth/create-session/no-protected-resources',
    area: 'Authentication',
    feature: 'createSession',
    title: 'An agent without protected resources can create a session without authenticating.',
    spec: '/specification/authentication#discovery',
    requires: [noProtectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      if ((selectedAgent(t, client).protectedResources?.length ?? 0) !== 0) {
        t.skip('The selected agent now advertises protected resources.');
      }
      await t.session(client);
    },
  },
  {
    id: 'auth/create-session/optional-resources-allow-unauthenticated',
    area: 'Authentication',
    feature: 'createSession',
    title: 'An agent with only optional protected resources can create a session without authenticating.',
    spec: '/specification/authentication#required-vs-optional-authentication',
    requires: [requires.protectedResources, onlyOptionalResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      if (protectedResources(t, client).some((resource) => resource.required !== false)) {
        t.skip('The selected agent now advertises a required protected resource.');
      }
      await t.session(client);
    },
  },
  {
    id: 'auth/create-session/required-resource-auth-required',
    area: 'Authentication',
    feature: 'createSession',
    title: 'SHOULD: A session creation rejected for missing authentication returns AuthRequired.',
    spec: '/specification/authentication#required-vs-optional-authentication',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      requiredResources(t, client);
      const channel = `ahp-session:/${t.uniqueId('unauthenticated-session')}`;
      try {
        await client.request('createSession', { channel, provider: t.profile.agent.provider });
      } catch (error) {
        assert.ok(error instanceof RpcError, 'session creation must fail with a JSON-RPC error');
        assert.equal(error.code, AhpErrorCodes.AuthRequired);
        return;
      }
      t.defer(() => client.request('disposeSession', { channel }));
      t.skip('Session allocation succeeded; authentication may be deferred until the agent is used.');
    },
  },
  {
    id: 'auth/create-session/auth-required-data-shape',
    area: 'Authentication',
    feature: 'createSession',
    title: 'An AuthRequired error includes a data object containing a resources array.',
    spec: '/reference/error-codes#authrequirederrordata',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      requiredResources(t, client);
      authRequiredResources(await observeAuthRequired(t, client));
    },
  },
  {
    id: 'auth/create-session/auth-required-lists-required-resource',
    area: 'Authentication',
    feature: 'createSession',
    title: 'An AuthRequired error identifies a required protected resource of the selected agent.',
    spec: '/specification/authentication#authrequired-error-code',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const required = requiredResources(t, client);
      const resources = authRequiredResources(await observeAuthRequired(t, client));
      assert.ok(resources.some((entry) => {
        assertRecord(entry);
        return required.some((resource) => resource.resource === entry.resource);
      }), 'AuthRequired must identify a resource that requires authentication');
    },
  },
  {
    id: 'auth/create-session/auth-required-resource-metadata',
    area: 'Authentication',
    feature: 'createSession',
    title: 'AuthRequired error resources carry complete protected resource metadata.',
    spec: '/specification/authentication#authrequired-error-code',
    requires: [requires.protectedResources],
    async run(t) {
      const client = await t.client({ authenticate: false });
      requiredResources(t, client);
      const resources = authRequiredResources(await observeAuthRequired(t, client));
      for (const resource of resources) assertResourceMetadata(resource);
    },
  },
  {
    id: 'auth/create-session/retry-after-authentication',
    area: 'Authentication',
    feature: 'createSession',
    title: 'A session creation rejected for authentication succeeds when retried after authenticating.',
    spec: '/specification/authentication#authrequired-error-code',
    requires: [requires.protectedResources, requires.authTokens],
    async run(t) {
      const client = await t.client({ authenticate: false });
      const tokens = requiredTokens(t, client);
      const channel = `ahp-session:/${t.uniqueId('authenticated-session')}`;
      await observeAuthRequired(t, client, { channel });
      await authenticateAll(client, tokens);
      await t.session(client, { channel });
    },
  },
];
