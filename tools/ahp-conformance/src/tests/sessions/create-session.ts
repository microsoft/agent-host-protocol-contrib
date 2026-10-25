import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { createSession, sessionState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/create-session/null-result',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'Creating a session returns null on success.',
    spec: '/reference/session#createsession',
    async run(t) {
      const client = await t.client();
      const session = await createSession(t, client);
      assert.equal(session.result, null);
    },
  },
  {
    id: 'sessions/create-session/duplicate-uri',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'Creating an existing session returns SessionAlreadyExists.',
    spec: '/reference/session#createsession',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await expectRpcError(client.request('createSession', {
        channel: session.sessionUri,
        provider: t.profile.agent.provider,
      }), AhpErrorCodes.SessionAlreadyExists);
    },
  },
  {
    id: 'sessions/create-session/unknown-provider',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'Creating a session with an unknown provider returns ProviderNotFound.',
    spec: '/reference/session#createsession',
    async run(t) {
      const client = await t.client();
      await expectRpcError(createSession(t, client, { provider: t.uniqueId('unknown-provider') }), AhpErrorCodes.ProviderNotFound);
    },
  },
  {
    id: 'sessions/create-session/eager-active-client',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'Creating a session can initialize the creating client as active.',
    spec: '/reference/session#createsession',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client, { activeClient: { clientId: client.clientId, tools: [] } });
      assert.ok(sessionState(client, session.sessionUri).activeClients.some((entry) => entry.clientId === client.clientId));
    },
  },
  {
    id: 'sessions/create-session/ready',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'Session creation reaches the ready lifecycle without requiring a turn.',
    spec: '/specification/session-channel#lifecycle',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await client.waitForState(session.sessionUri,
        () => sessionState(client, session.sessionUri).lifecycle !== 'creating');
      assert.equal(sessionState(client, session.sessionUri).lifecycle, 'ready');
    },
  },
  {
    id: 'sessions/create-session/mirror-consistency',
    area: 'Root & sessions',
    feature: 'createSession',
    title: 'The session mirror matches a fresh snapshot after initialization.',
    spec: '/specification/subscriptions#subscribe-request',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
];
