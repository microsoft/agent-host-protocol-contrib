import type { ChatState, MoveChatResult, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';
import { chatState, completeTurn, createChat, userMessage } from './helpers.ts';

async function movableChat(t: TestContext, client: Client, session: SessionHandle): Promise<string> {
  const chat = await createChat(t, client, session);
  if (chatState(client, chat).movable !== true) t.skip('The created chat does not advertise movable: true.');
  return chat;
}

async function moveIfSupported(t: TestContext, client: Client, chat: string, session: string, after?: string) {
  try {
    return await client.request<MoveChatResult>('moveChat', {
      channel: chat, destination: { kind: 'session', session, ...(after ? { after } : {}) },
    });
  } catch (error) {
    if (error instanceof RpcError) t.skip(`The host declined this move destination or source condition: ${error.message}`);
    throw error;
  }
}

export const tests: TestDefinition[] = [
  {
    id: 'chats/create-chat/catalog-before-response',
    area: 'Chats & turns', feature: 'createChat',
    title: 'Creating a chat publishes its session catalog entry before responding.',
    spec: '/specification/chat-channel#creation', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = `ahp-chat:/${t.uniqueId('chat')}`;
      await client.request('createChat', { channel: session.sessionUri, chat });
      assert.ok(client.actions(session.sessionUri).some((e) => e.action.type === 'session/chatAdded' && e.action.summary.resource === chat));
      assert.ok(client.state<SessionState>(session.sessionUri)?.chats.some((entry) => entry.resource === chat));
    },
  },
  {
    id: 'chats/create-chat/empty-visible-history',
    area: 'Chats & turns', feature: 'createChat',
    title: 'A new chat without a source or initial message has empty visible history.',
    spec: '/specification/chat-channel#creation', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = await createChat(t, client, session);
      assert.deepEqual(chatState(client, chat).turns, []);
      assert.equal(chatState(client, chat).activeTurn, undefined);
    },
  },
  {
    id: 'chats/create-chat/independent-context',
    area: 'Chats & turns', feature: 'createChat',
    title: 'Independent chats do not overwrite one another’s remembered context.',
    spec: '/specification/chat-channel#active-chat', requires: [requires.multipleChats, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = await createChat(t, client, session);
      const original = t.uniqueId('original');
      await completeTurn(session, prompts.remember(original));
      await completeTurn(session, prompts.remember(t.uniqueId('other')), chat);
      const recalled = await completeTurn(session, prompts.recall());
      assert.ok(session.responseText(recalled.id).trim().includes(original));
    },
  },
  {
    id: 'chats/create-chat/initial-message-starts-turn',
    area: 'Chats & turns', feature: 'createChat',
    title: 'An initial message becomes the new chat’s first visible turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.multipleChats, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const message = userMessage(prompts.reply(t.uniqueId('reply')));
      const chat = await createChat(t, client, session, { initialMessage: message });
      const state = await client.waitForState<ChatState>(chat, (s) => !!s.activeTurn || s.turns.length > 0);
      const turn = state.activeTurn ?? state.turns[0];
      assert.equal(turn.message.text, message.text);
      await session.waitForTurnEnd(turn.id, { chatUri: chat });
    },
  },
  {
    id: 'chats/move-chat/default-not-movable',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'The default chat does not advertise move eligibility.',
    spec: '/specification/chat-channel#moving-chats',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      assert.notEqual(chatState(client, session.chatUri).movable, true);
      assert.notEqual(client.state<SessionState>(session.sessionUri)?.chats.find((entry) => entry.resource === session.chatUri)?.movable, true);
    },
  },
  {
    id: 'chats/move-chat/reorder-after-default',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'A successful same-session move places the chat immediately after its anchor.',
    spec: '/specification/chat-channel#moving-chats', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = await movableChat(t, client, session);
      await createChat(t, client, session);
      await moveIfSupported(t, client, chat, session.sessionUri);
      await moveIfSupported(t, client, chat, session.sessionUri, session.chatUri);
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = await observer.subscribe(session.sessionUri);
      const order = (snapshot.state as SessionState).chats.map((entry) => entry.resource);
      assert.equal(order[order.indexOf(session.chatUri) + 1], chat);
    },
  },
  {
    id: 'chats/move-chat/self-anchor-rejected',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'A move that anchors a chat to itself is rejected.',
    spec: '/specification/chat-channel#validation', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = await movableChat(t, client, session);
      await assert.rejects(client.request('moveChat', {
        channel: chat, destination: { kind: 'session', session: session.sessionUri, after: chat },
      }), RpcError);
    },
  },
  {
    id: 'chats/move-chat/rejection-preserves-catalog',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'A rejected move leaves the session catalog unchanged.',
    spec: '/specification/chat-channel#validation', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const chat = await movableChat(t, client, session);
      const before = (await client.subscribe(session.sessionUri)).state as SessionState;
      await assert.rejects(client.request('moveChat', {
        channel: chat, destination: { kind: 'session', session: session.sessionUri, after: chat },
      }), RpcError);
      const observer = await t.client({ initialSubscriptions: [] });
      const after = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.deepEqual(after.chats, before.chats);
    },
  },
  {
    id: 'chats/move-chat/ownership-transferred',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'A successful cross-session move transfers the chat’s catalog ownership.',
    spec: '/specification/chat-channel#atomic-synchronization', requires: [requires.multipleChats],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const destination = await t.session(client);
      const chat = await movableChat(t, client, source);
      const result = await moveIfSupported(t, client, chat, destination.sessionUri);
      assert.equal(result.session, destination.sessionUri);
      const observer = await t.client({ initialSubscriptions: [] });
      const before = (await observer.subscribe(source.sessionUri)).state as SessionState;
      const after = (await observer.subscribe(destination.sessionUri)).state as SessionState;
      assert.ok(!before.chats.some((entry) => entry.resource === chat));
      assert.ok(after.chats.some((entry) => entry.resource === chat));
    },
  },
  {
    id: 'chats/move-chat/subscription-preserved',
    area: 'Chats & turns', feature: 'moveChat',
    title: 'A chat subscription remains usable after an ownership move.',
    spec: '/specification/chat-channel#moving-chats', requires: [requires.multipleChats, requires.model],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const destination = await t.session(client);
      const chat = await movableChat(t, client, source);
      await moveIfSupported(t, client, chat, destination.sessionUri);
      const reply = t.uniqueId('after-move');
      const turn = await completeTurn(source, prompts.reply(reply), chat);
      assert.ok(source.responseText(turn.id, { chatUri: chat }).trim().includes(reply));
    },
  },
];
