import type { ChatState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { assertChatMirrorMatchesSnapshot, chatSource, chatState, completeTurn, createChat } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'chats/fork/copies-visible-history',
    area: 'Chats & turns', feature: 'fork',
    title: 'A fork copies visible source history through the referenced completed turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const first = await completeTurn(session, prompts.reply(t.uniqueId('first')));
      const second = await completeTurn(session, prompts.reply(t.uniqueId('second')));
      const fork = await createChat(t, client, session, { source: chatSource('fork', session.chatUri, second.id) });
      // Copying visible history does not require preserving source turn ids.
      const visibleHistory = (turns: ChatState['turns']) => turns.map(({ id: _id, ...turn }) => turn);
      assert.deepEqual(visibleHistory(chatState(client, fork).turns), visibleHistory([first, second]));
    },
  },
  {
    id: 'chats/fork/recalls-source-context',
    area: 'Chats & turns', feature: 'fork',
    title: 'A fork receives the referenced source history as model context.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('fork-memory');
      const source = await completeTurn(session, prompts.remember(phrase));
      const fork = await createChat(t, client, session, { source: chatSource('fork', session.chatUri, source.id) });
      const recalled = await completeTurn(session, prompts.recall(), fork);
      assert.ok(session.responseText(recalled.id, { chatUri: fork }).trim().includes(phrase));
    },
  },
  {
    id: 'chats/fork/excludes-later-visible-turns',
    area: 'Chats & turns', feature: 'fork',
    title: 'A fork excludes source turns later than the referenced cutoff.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const first = await completeTurn(session, prompts.reply(t.uniqueId('first')));
      const later = await completeTurn(session, prompts.reply(t.uniqueId('later')));
      const fork = await createChat(t, client, session, { source: chatSource('fork', session.chatUri, first.id) });
      assert.ok(!chatState(client, fork).turns.some((turn) => turn.id === later.id));
    },
  },
  {
    id: 'chats/fork/excludes-later-model-context',
    area: 'Chats & turns', feature: 'fork',
    title: 'A fork does not receive model context from source turns beyond its cutoff.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('cutoff-memory');
      const first = await completeTurn(session, prompts.remember(phrase));
      await completeTurn(session, prompts.remember(t.uniqueId('later-memory')));
      const fork = await createChat(t, client, session, { source: chatSource('fork', session.chatUri, first.id) });
      const recalled = await completeTurn(session, prompts.recall(), fork);
      assert.ok(session.responseText(recalled.id, { chatUri: fork }).trim().includes(phrase));
    },
  },
  {
    id: 'chats/fork/unknown-turn-rejected',
    area: 'Chats & turns', feature: 'fork',
    title: 'Fork creation rejects an unknown source turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('fork')}`,
        source: chatSource('fork', session.chatUri, t.uniqueId('missing')),
      }), RpcError);
    },
  },
  {
    id: 'chats/fork/unknown-chat-rejected',
    area: 'Chats & turns', feature: 'fork',
    title: 'Fork creation rejects an unknown source chat.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('fork')}`,
        source: chatSource('fork', `ahp-chat:/${t.uniqueId('missing')}`, t.uniqueId('turn')),
      }), RpcError);
    },
  },
  {
    id: 'chats/fork/cross-session-source-rejected',
    area: 'Chats & turns', feature: 'fork',
    title: 'Fork creation rejects a source chat outside the target session.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const target = await t.session(client);
      const turn = await completeTurn(source, prompts.reply(t.uniqueId('source')));
      await target.ready();
      await assert.rejects(client.request('createChat', {
        channel: target.sessionUri, chat: `ahp-chat:/${t.uniqueId('fork')}`,
        source: chatSource('fork', source.chatUri, turn.id),
      }), RpcError);
    },
  },
  {
    id: 'chats/fork/mirror-matches-snapshot',
    area: 'Chats & turns', feature: 'fork',
    title: 'The fork mirror matches a fresh snapshot after its first new turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const source = await completeTurn(session, prompts.reply(t.uniqueId('source')));
      const fork = await createChat(t, client, session, { source: chatSource('fork', session.chatUri, source.id) });
      await completeTurn(session, prompts.reply(t.uniqueId('fork-reply')), fork);
      await assertChatMirrorMatchesSnapshot(t, client, session, fork);
    },
  },
  {
    id: 'chats/side-chat/source-context',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'A side chat receives the referenced source turn as model context.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('side-memory');
      const source = await completeTurn(session, prompts.remember(phrase));
      const side = await createChat(t, client, session, { source: chatSource('sideChat', session.chatUri, source.id) });
      const recalled = await completeTurn(session, prompts.recall(), side);
      assert.ok(session.responseText(recalled.id, { chatUri: side }).trim().includes(phrase));
    },
  },
  {
    id: 'chats/side-chat/empty-visible-history',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'A side chat does not copy the source transcript into its visible history.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const source = await completeTurn(session, prompts.reply(t.uniqueId('source')));
      const side = await createChat(t, client, session, { source: chatSource('sideChat', session.chatUri, source.id) });
      assert.deepEqual(chatState(client, side).turns, []);
    },
  },
  {
    id: 'chats/side-chat/excludes-later-model-context',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'A side chat receives source context only through its referenced turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('source-cutoff');
      const first = await completeTurn(session, prompts.remember(phrase));
      await completeTurn(session, prompts.remember(t.uniqueId('later-source')));
      const side = await createChat(t, client, session, { source: chatSource('sideChat', session.chatUri, first.id) });
      const recalled = await completeTurn(session, prompts.recall(), side);
      assert.ok(session.responseText(recalled.id, { chatUri: side }).trim().includes(phrase));
    },
  },
  {
    id: 'chats/side-chat/selection-preserved',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'A side chat preserves the exact selected-text snapshot in its origin.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const text = t.uniqueId('selected');
      const source = await completeTurn(session, prompts.reply(text));
      const part = source.responseParts.find((entry) => entry.kind === 'markdown');
      assert.ok(part && part.kind === 'markdown');
      const selection = { text, responsePartId: part.id };
      const side = await createChat(t, client, session, {
        source: chatSource('sideChat', session.chatUri, source.id, selection),
      });
      const observer = await t.client({ initialSubscriptions: [] });
      const state = (await observer.subscribe(side)).state as ChatState;
      assert.ok(state.origin?.kind === 'sideChat');
      assert.deepEqual(state.origin.selection, selection);
    },
  },
  {
    id: 'chats/side-chat/empty-selection-rejected',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'Side-chat creation rejects an empty selected-text snapshot.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const source = await completeTurn(session, prompts.reply(t.uniqueId('source')));
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('side')}`,
        source: chatSource('sideChat', session.chatUri, source.id, { text: '' }),
      }), RpcError);
    },
  },
  {
    id: 'chats/side-chat/unknown-turn-rejected',
    area: 'Chats & turns', feature: 'sideChat',
    title: 'Side-chat creation rejects an unknown source turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('side')}`,
        source: chatSource('sideChat', session.chatUri, t.uniqueId('missing')),
      }), RpcError);
    },
  },
];
