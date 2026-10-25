import type { SessionActiveClient, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, jsonValue } from './coverage-helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/active-clients/upsert-replaces-contributions',
    area: 'Root & sessions', feature: 'activeClients',
    title: 'Refreshing an active client replaces its contributions without duplicating or evicting clients.',
    spec: '/reference/session#sessionactiveclientsetaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const other = await t.client({ initialSubscriptions: [] });
      await other.subscribe(session.sessionUri);
      const peer: SessionActiveClient = { clientId: other.clientId, tools: [] };
      await accepted(other, session.sessionUri, { type: 'session/activeClientSet', activeClient: peer });
      await accepted(client, session.sessionUri, {
        type: 'session/activeClientSet', activeClient: {
          clientId: client.clientId, displayName: 'Original test client', customizations: [],
          tools: [{ name: t.uniqueId('old-tool'), description: 'Inert test registration.', inputSchema: { type: 'object' } }],
        },
      });
      const replacement: SessionActiveClient = { clientId: client.clientId, tools: [] };
      await accepted(client, session.sessionUri, { type: 'session/activeClientSet', activeClient: replacement });
      const snapshot = (await other.subscribe(session.sessionUri)).state as SessionState;
      const entries = snapshot.activeClients.filter((entry) => entry.clientId === client.clientId);
      assert.equal(entries.length, 1);
      assert.deepEqual(jsonValue(entries[0]), jsonValue(replacement));
      assert.deepEqual(jsonValue(snapshot.activeClients.find((entry) => entry.clientId === other.clientId)), jsonValue(peer));
    },
  },
  {
    id: 'sessions/active-clients/unsubscribe-removes-membership',
    area: 'Root & sessions', feature: 'activeClients',
    title: 'SHOULD: Unsubscribing an active client removes its membership for remaining subscribers.',
    spec: '/guide/customizations#active-client-lifecycle',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [] });
      await observer.subscribe(session.sessionUri);
      await accepted(client, session.sessionUri, {
        type: 'session/activeClientSet', activeClient: { clientId: client.clientId, tools: [] },
      }, t);
      const after = observer.lastServerSeq;
      await client.unsubscribe(session.sessionUri);
      await client.request('ping', { channel: 'ahp-root://' });
      await observer.waitForAction(session.sessionUri, (event) => event.serverSeq > after &&
        event.action.type === 'session/activeClientRemoved' && event.action.clientId === client.clientId &&
        event.rejectionReason === undefined, { timeoutMs: 5_000 });
      const fresh = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.ok(!fresh.activeClients.some((entry) => entry.clientId === client.clientId));
    },
  },
];
