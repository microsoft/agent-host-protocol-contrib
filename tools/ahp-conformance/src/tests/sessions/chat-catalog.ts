import type { ChatState, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { additionalChat, successful, summaryUpdate } from './coverage-helpers.ts';
import { sessionState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/chat-catalog/compact-status-exact',
    area: 'Root & sessions', feature: 'chatCatalog',
    title: 'A changed chat status is projected exactly into the complete compact root catalog.',
    spec: '/specification/session-channel#chat-catalog-mutations', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const observer = await t.client();
      const since = observer.received.length;
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      const fresh = (await client.subscribe(session.sessionUri)).state as SessionState;
      const chat = (await client.subscribe(session.chatUri)).state as ChatState;
      const event = await summaryUpdate(observer, session.sessionUri, since, (changes) =>
        changes.chats?.some((entry) => entry.resource === session.chatUri && entry.status === chat.status) === true);
      assert.deepEqual(event.changes.chats?.map((entry) => entry.resource), fresh.chats.map((entry) => entry.resource));
      for (const entry of event.changes.chats ?? []) {
        assert.equal(entry.status, fresh.chats.find((chat) => chat.resource === entry.resource)?.status);
      }
    },
  },
  {
    id: 'sessions/chat-catalog/update-omits-resource',
    area: 'Root & sessions', feature: 'chatCatalog',
    title: 'Host-emitted chat catalog patches omit the immutable resource field.',
    spec: '/reference/session#sessionchatupdatedaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const after = client.lastServerSeq;
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      await client.waitForAction(session.sessionUri, (event) => event.serverSeq > after && successful(event) &&
        event.action.type === 'session/chatUpdated', { timeoutMs: 5_000 });
      await client.request('ping', { channel: 'ahp-root://' });
      const updates = client.actions(session.sessionUri).filter((event) => event.serverSeq > after && successful(event) &&
        event.action.type === 'session/chatUpdated');
      for (const event of updates) {
        if (event.action.type === 'session/chatUpdated') assert.ok(!Object.hasOwn(event.action.changes, 'resource'));
      }
    },
  },
  {
    id: 'sessions/chat-catalog/reorder-complete-bijection',
    area: 'Root & sessions', feature: 'chatCatalog',
    title: 'An accepted chat move emits a complete duplicate-free catalog order.',
    spec: '/reference/session#sessionchatsreorderedaction', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const chat = await additionalChat(t, client, session);
      if (client.state<ChatState>(chat)?.movable !== true) t.skip('The created chat does not advertise movable: true.');
      const before = ((await client.subscribe(session.sessionUri)).state as SessionState).chats.map((entry) => entry.resource);
      if (before[0] === chat) t.skip('The movable chat is already first; this setup cannot induce an order change.');
      const after = client.lastServerSeq;
      try {
        await client.request('moveChat', { channel: chat, destination: { kind: 'session', session: session.sessionUri } });
      } catch (error) {
        if (error instanceof RpcError) t.skip(`The host declined the move destination/source condition: ${error.message}`);
        throw error;
      }
      const event = await client.waitForAction(session.sessionUri, (event) => event.serverSeq > after && successful(event) &&
        event.action.type === 'session/chatsReordered', { timeoutMs: 5_000 });
      assert.ok(event.action.type === 'session/chatsReordered');
      const order = event.action.chats;
      assert.equal(new Set(order).size, before.length, 'reorder must contain every chat exactly once');
      assert.deepEqual([...order].sort(), [...before].sort());
      const observer = await t.client({ initialSubscriptions: [] });
      const fresh = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.deepEqual(order, fresh.chats.map((entry) => entry.resource));
    },
  },
  {
    id: 'sessions/default-chat/change-preserves-catalog-order',
    area: 'Root & sessions', feature: 'defaultChat',
    title: 'Changing the default chat preserves the authoritative catalog order.',
    spec: '/specification/session-channel#chat-catalog-mutations', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const chat = await additionalChat(t, client, session);
      const before = sessionState(client, session.sessionUri);
      if (before.defaultChat === chat) t.skip('The new chat is already designated as default.');
      const order = before.chats.map((entry) => entry.resource);
      const seq = client.dispatch(session.sessionUri, { type: 'session/defaultChatChanged', defaultChat: chat });
      const echo = await client.waitForAction(session.sessionUri, (event) =>
        event.origin?.clientId === client.clientId && event.origin.clientSeq === seq);
      if (echo.rejectionReason !== undefined) {
        return t.skip('Host rejected client-dispatched session/defaultChatChanged (not client-dispatchable in 1.0.0 action-origin)');
      }
      const observer = await t.client({ initialSubscriptions: [] });
      const fresh = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.equal(fresh.defaultChat, chat);
      assert.deepEqual(fresh.chats.map((entry) => entry.resource), order);
    },
  },
];
