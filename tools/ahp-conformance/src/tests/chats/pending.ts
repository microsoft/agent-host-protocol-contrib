import type { ChatState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, TestContext, TestDefinition } from '../../harness/types.ts';
import { accepted, assertChatMirrorMatchesSnapshot, chatState, pendingKind, userMessage } from './helpers.ts';

async function pendingSnapshot(client: Client, chat: string) {
  const snapshot = await client.subscribe(chat);
  return snapshot.state as ChatState;
}

async function queuedChat(t: TestContext) {
  const client = await t.client();
  const session = await t.session(client);
  const text = Array.from({ length: 128 }, (_, i) => `${t.uniqueId('busy')}-${i}`).join(' ');
  const turnId = session.startTurn(prompts.reply(text, { chunks: 32, waitMs: 100 }));
  await client.waitForAction(session.chatUri, (e) => e.action.type === 'chat/turnStarted' && e.action.turnId === turnId);
  return {
    client, chat: session.chatUri,
    async snapshot() {
      const state = await pendingSnapshot(client, session.chatUri);
      if (state.activeTurn?.id !== turnId) t.skip('The setup turn ended before the queued-message snapshot was taken.');
      return state.queuedMessages ?? [];
    },
  };
}

export const tests: TestDefinition[] = [
  {
    id: 'chats/pending-message-set/idle-steering-retained',
    area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'An idle chat retains a steering message in its separate slot.',
    spec: '/specification/chat-channel#steering-messages',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const id = t.uniqueId('pending');
      const message = userMessage(t.uniqueId('steering'));
      await accepted(client, session.chatUri, { type: 'chat/pendingMessageSet', id, kind: pendingKind('steering'), message });
      const entry = (await pendingSnapshot(client, session.chatUri)).steeringMessage;
      assert.ok(entry, 'idle steering message was not retained');
      assert.deepEqual({ id: entry.id, message: entry.message }, { id, message });
    },
  },
  {
    id: 'chats/pending-message-set/steering-replaces-slot',
    area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'A later steering message replaces the single steering slot even with a different id.',
    spec: '/reference/chat#chatpendingmessagesetaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const ids = [t.uniqueId('first'), t.uniqueId('second')];
      for (const id of ids) await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('steering'), message: userMessage(id),
      });
      const entry = (await pendingSnapshot(client, session.chatUri)).steeringMessage;
      assert.ok(entry, 'missing replacement steering message');
      assert.deepEqual({ id: entry.id, message: entry.message }, {
        id: ids[1], message: userMessage(ids[1]),
      });
    },
  },
  {
    id: 'chats/pending-message-set/upsert-in-place',
    area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'Updating a queued id replaces its message without changing its queue position.',
    spec: '/reference/chat#chatpendingmessagesetaction', requires: [requires.model],
    async run(t) {
      const { client, chat, snapshot } = await queuedChat(t);
      const ids = [t.uniqueId('first'), t.uniqueId('second')];
      for (const id of ids) await accepted(client, chat, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('queued'), message: userMessage(id),
      });
      const message = userMessage(t.uniqueId('updated'));
      await accepted(client, chat, { type: 'chat/pendingMessageSet', id: ids[0], kind: pendingKind('queued'), message });
      const entries = await snapshot();
      assert.deepEqual(entries.map((entry) => entry.id), ids);
      assert.deepEqual(entries[0].message, message);
    },
  },
  {
    id: 'chats/pending-message-removed/matching-id-and-kind',
    area: 'Chats & turns', feature: 'pendingMessageRemoved',
    title: 'Removing the current steering message clears its separate slot.',
    spec: '/reference/chat#chatpendingmessageremovedaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const id = t.uniqueId('steering');
      await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('steering'), message: userMessage(id),
      });
      await accepted(client, session.chatUri, { type: 'chat/pendingMessageRemoved', id, kind: pendingKind('steering') });
      assert.equal((await pendingSnapshot(client, session.chatUri)).steeringMessage, undefined);
    },
  },
  {
    id: 'chats/queued-messages-reordered/explicit-order',
    area: 'Chats & turns', feature: 'queuedMessagesReordered',
    title: 'Reordering queued messages persists the requested queue order.',
    spec: '/reference/chat#chatqueuedmessagesreorderedaction', requires: [requires.model],
    async run(t) {
      const { client, chat, snapshot } = await queuedChat(t);
      const ids = [t.uniqueId('first'), t.uniqueId('second'), t.uniqueId('third')];
      for (const id of ids) await accepted(client, chat, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('queued'), message: userMessage(id),
      });
      const order = [ids[2], ids[0], ids[1]];
      await accepted(client, chat, { type: 'chat/queuedMessagesReordered', order });
      assert.deepEqual((await snapshot()).map((entry) => entry.id), order);
    },
  },
  {
    id: 'chats/queued-messages-reordered/unmentioned-entries-retained',
    area: 'Chats & turns', feature: 'queuedMessagesReordered',
    title: 'A partial reorder appends unmentioned queued messages in their original relative order.',
    spec: '/reference/chat#chatqueuedmessagesreorderedaction', requires: [requires.model],
    async run(t) {
      const { client, chat, snapshot } = await queuedChat(t);
      const ids = [t.uniqueId('first'), t.uniqueId('second'), t.uniqueId('third')];
      for (const id of ids) await accepted(client, chat, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('queued'), message: userMessage(id),
      });
      await accepted(client, chat, { type: 'chat/queuedMessagesReordered', order: [ids[2]] });
      assert.deepEqual((await snapshot()).map((entry) => entry.id), [ids[2], ids[0], ids[1]]);
    },
  },
  {
    id: 'chats/pending-message-set/mirror-matches-snapshot',
    area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'Steering-slot updates leave the reducer mirror consistent with a fresh snapshot.',
    spec: '/reference/chat#chatpendingmessagesetaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const ids = [t.uniqueId('first'), t.uniqueId('second')];
      for (const id of ids) await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('steering'), message: userMessage(id),
      });
      await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', id: ids[1], kind: pendingKind('steering'), message: userMessage(t.uniqueId('updated')),
      });
      await accepted(client, session.chatUri, { type: 'chat/pendingMessageRemoved', id: ids[1], kind: pendingKind('steering') });
      await assertChatMirrorMatchesSnapshot(t, client, session);
    },
  },
  {
    id: 'chats/pending-message-set/queued-idle-consumption',
    area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'SHOULD: An idle chat consumes queued input as a turn with the same pending id.',
    spec: '/specification/chat-channel#queued-messages', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const id = t.uniqueId('queued');
      await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', id, kind: pendingKind('queued'), message: userMessage(prompts.reply(t.uniqueId('reply'))),
      });
      const envelope = await client.waitForAction(session.chatUri, (e) =>
        e.action.type === 'chat/turnStarted' && e.action.queuedMessageId === id);
      assert.ok(envelope.action.type === 'chat/turnStarted');
      await session.waitForTurnEnd(envelope.action.turnId);
      assert.ok(!chatState(client, session.chatUri).queuedMessages?.some((entry) => entry.id === id));
    },
  },
  {
    id: 'chats/draft-changed/set-visible-in-snapshot',
    area: 'Chats & turns', feature: 'draftChanged',
    title: 'A synchronized draft is visible in a fresh chat snapshot.',
    spec: '/reference/chat#chatdraftchangedaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const draft = userMessage(t.uniqueId('draft'));
      await accepted(client, session.chatUri, { type: 'chat/draftChanged', draft });
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = await observer.subscribe(session.chatUri);
      assert.deepEqual((snapshot.state as ChatState).draft, draft);
    },
  },
  {
    id: 'chats/draft-changed/clear-visible-in-snapshot',
    area: 'Chats & turns', feature: 'draftChanged',
    title: 'Clearing a synchronized draft removes it from fresh snapshots.',
    spec: '/reference/chat#chatdraftchangedaction',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await accepted(client, session.chatUri, { type: 'chat/draftChanged', draft: userMessage(t.uniqueId('draft')) });
      await accepted(client, session.chatUri, { type: 'chat/draftChanged' });
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = await observer.subscribe(session.chatUri);
      assert.equal((snapshot.state as ChatState).draft, undefined);
    },
  },
];
