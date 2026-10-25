import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { RootState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { assertRecord, assertResourceMetadata, assertUrl, authRequiredResources, observeAuthNotification } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'auth/auth-required/any-command-error-data',
    area: 'Authentication',
    feature: 'AuthRequired',
    title: 'AuthRequired errors from commands other than createSession carry protected resource metadata.',
    spec: '/specification/authentication#authrequired-error-code',
    async run(t) {
      const client = await t.client({ authenticate: false });
      const resources = client.state<RootState>('ahp-root://')?.agents.flatMap((agent) => agent.protectedResources ?? []) ?? [];
      const probes: { method: string; params: unknown }[] = [
        { method: 'resolveSessionConfig', params: { channel: 'ahp-root://', provider: t.profile.agent.provider } },
        { method: 'listSessions', params: { channel: 'ahp-root://' } },
        ...resources.map((resource) => ({
          method: 'authenticate',
          params: { channel: 'ahp-root://', resource: resource.resource, token: t.uniqueId('invalid-token') },
        })),
      ];
      let observed = 0;
      for (const probe of probes) {
        try {
          await client.request(probe.method, probe.params);
        } catch (error) {
          if (!(error instanceof RpcError)) throw error;
          if (error.code !== AhpErrorCodes.AuthRequired) continue;
          observed++;
          for (const resource of authRequiredResources(error)) assertResourceMetadata(resource);
        }
      }
      if (observed === 0) t.skip('No non-createSession probe returned the optional AuthRequired error code.');
    },
  },
  {
    id: 'auth/auth-required/resource-metadata',
    area: 'Authentication',
    feature: 'auth/required',
    title: 'An emitted auth/required notification includes complete protected resource metadata.',
    spec: '/reference/common#authrequired',
    requires: [requires.protectedResources],
    async run(t) {
      const params = await observeAuthNotification(t);
      assertRecord(params);
      assertResourceMetadata(params.resource);
    },
  },
  {
    id: 'auth/auth-required/channel-uri',
    area: 'Authentication',
    feature: 'auth/required',
    title: 'An emitted auth/required notification identifies its channel with a URI.',
    spec: '/reference/common#authrequired',
    requires: [requires.protectedResources],
    async run(t) {
      const params = await observeAuthNotification(t);
      assertRecord(params);
      assertUrl(params.channel);
    },
  },
  {
    id: 'auth/auth-required/reason-string',
    area: 'Authentication',
    feature: 'auth/required',
    title: 'An emitted auth/required reason is a string when supplied.',
    spec: '/reference/common#authrequiredreason',
    requires: [requires.protectedResources],
    async run(t) {
      const params = await observeAuthNotification(t);
      assertRecord(params);
      if (params.reason === undefined) t.skip('The auth/required notification omits its optional reason.');
      assert.equal(typeof params.reason, 'string');
    },
  },
];
