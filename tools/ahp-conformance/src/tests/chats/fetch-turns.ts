import { JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import type { ChatState, ChatTurnsLoadedAction, Snapshot } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, TestContext, TestDefinition } from '../../harness/types.ts';
import { chatState, completeTurn } from './helpers.ts';

async function windowedHistory(t: TestContext) {
  const client = await t.client();
  const session = await t.session(client);
  for (let i = 0; i < 3; i++) await completeTurn(session, prompts.reply(t.uniqueId('history')));
  const retained = chatState(client, session.chatUri).turns;
  const reader = await t.client({ initialSubscriptions: [] });
  const result = await reader.request<{ snapshot: Snapshot }>('subscribe', { channel: session.chatUri, view: { turns: 1 } });
  assert.ok(result.snapshot, 'a stateful chat subscription must return a snapshot');
  const state = result.snapshot.state as ChatState;
  if (state.turnsNextCursor === undefined) t.skip('The host returned all retained turns despite the advisory tail view.');
  return { reader, uri: session.chatUri, state, retained };
}

async function fetchPage(reader: Client, uri: string, cursor: string): Promise<ChatTurnsLoadedAction[]> {
  const before = reader.actions(uri).length;
  await reader.request('fetchTurns', { channel: uri, cursor });
  const loaded = reader.actions(uri).slice(before).flatMap((e) =>
    e.action.type === 'chat/turnsLoaded' ? [e.action] : []);
  assert.ok(loaded.length > 0, 'fetchTurns must dispatch chat/turnsLoaded before responding');
  return loaded;
}

export const tests: TestDefinition[] = [
  {
    id: 'chats/fetch-turns/invalid-cursor',
    area: 'Chats & turns', feature: 'fetchTurns',
    title: 'Fetching turns with an unrecognized cursor returns InvalidParams.',
    spec: '/reference/session#fetchturns',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await expectRpcError(client.request('fetchTurns', {
        channel: session.chatUri, cursor: t.uniqueId('invalid-cursor'),
      }), JsonRpcErrorCodes.InvalidParams);
    },
  },
  {
    id: 'chats/fetch-turns/unwindowed-snapshot-has-retained-turns',
    area: 'Chats & turns', feature: 'fetchTurns',
    title: 'A chat snapshot requested without a view contains every retained completed turn.',
    spec: '/specification/chat-channel#state', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      for (let i = 0; i < 2; i++) await completeTurn(session, prompts.reply(t.uniqueId('retained')));
      const retained = chatState(client, session.chatUri).turns;
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = await observer.subscribe(session.chatUri);
      const state = snapshot.state as ChatState;
      assert.deepEqual(state.turns.map((turn) => turn.id), retained.map((turn) => turn.id));
      assert.equal(state.turnsNextCursor, undefined);
    },
  },
  {
    id: 'chats/fetch-turns/loads-before-response',
    area: 'Chats & turns', feature: 'fetchTurns',
    title: 'Fetching an older page publishes the loaded completed turns before responding.',
    spec: '/reference/session#fetchturns', requires: [requires.model],
    async run(t) {
      const { reader, uri, state, retained } = await windowedHistory(t);
      assert.ok(state.turnsNextCursor !== undefined);
      const pages = await fetchPage(reader, uri, state.turnsNextCursor);
      const ids = new Set(state.turns.map((turn) => turn.id));
      const loaded = pages.flatMap((page) => page.turns);
      assert.ok(loaded.length > 0, 'a cursor advertising older turns must load older turns');
      for (const turn of loaded) {
        assert.ok(!ids.has(turn.id), 'a loaded page must precede the existing window');
        assert.ok(retained.some((entry) => entry.id === turn.id), 'loaded turn was not retained history');
      }
    },
  },
  {
    id: 'chats/fetch-turns/pagination-prepends-history',
    area: 'Chats & turns', feature: 'fetchTurns',
    title: 'Successive pages prepend retained turns in chronological order until the cursor clears.',
    spec: '/reference/chat#chatturnsloadedaction', requires: [requires.model],
    async run(t) {
      const { reader, uri, state, retained } = await windowedHistory(t);
      let turns = state.turns;
      let cursor = state.turnsNextCursor;
      for (let page = 0; cursor !== undefined; page++) {
        assert.ok(page < retained.length + 1, 'pagination did not finish within the retained history size');
        for (const loaded of await fetchPage(reader, uri, cursor)) {
          turns = [...loaded.turns, ...turns];
          cursor = loaded.turnsNextCursor;
        }
      }
      assert.deepEqual(turns.map((turn) => turn.id), retained.map((turn) => turn.id));
    },
  },
];
