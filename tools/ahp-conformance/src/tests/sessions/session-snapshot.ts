import type { SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { sessionState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/session-snapshot/metadata',
    area: 'Root & sessions',
    feature: 'sessionSnapshot',
    title: 'A session snapshot contains its required flat metadata.',
    spec: '/reference/session#sessionmetadata',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const snapshot = await client.subscribe(session.sessionUri);
      assert.equal(snapshot.resource, session.sessionUri);
      const state = snapshot.state as SessionState;
      assert.equal(state.provider, t.profile.agent.provider);
      assert.equal(typeof state.title, 'string');
      assert.ok(Number.isInteger(state.status));
    },
  },
  {
    id: 'sessions/session-snapshot/active-clients',
    area: 'Root & sessions',
    feature: 'sessionSnapshot',
    title: 'A session snapshot contains an active-clients array.',
    spec: '/reference/session#sessionstate',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      assert.ok(Array.isArray(sessionState(client, session.sessionUri).activeClients));
    },
  },
  {
    id: 'sessions/session-snapshot/default-chat-catalogued',
    area: 'Root & sessions',
    feature: 'sessionSnapshot',
    title: 'The initial chat is present in the session chat catalog.',
    spec: '/reference/root#agentcapabilities',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      assert.ok(sessionState(client, session.sessionUri).chats.some((chat) => chat.resource === session.chatUri));
    },
  },
  {
    id: 'sessions/session-snapshot/default-chat-not-movable',
    area: 'Root & sessions',
    feature: 'sessionSnapshot',
    title: 'A designated default chat is not advertised as movable.',
    spec: '/specification/session-channel#chat-catalog-mutations',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const state = sessionState(client, session.sessionUri);
      if (state.defaultChat === undefined) t.skip('Session does not designate a default chat.');
      const chat = state.chats.find((entry) => entry.resource === state.defaultChat);
      assert.ok(chat, 'defaultChat must refer to a catalog entry');
      assert.notEqual(chat.movable, true);
    },
  },
];
