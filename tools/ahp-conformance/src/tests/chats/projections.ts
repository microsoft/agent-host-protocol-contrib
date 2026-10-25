import type { ChatState, ChatSummary, ListSessionsResult, SessionState, SessionSummary, SessionSummaryChangedParams } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';
import { chatState, completeTurn, createChat, normalized, snapshotState } from './helpers.ts';

const IS_READ = 1 << 5;
const summaryFields = ['resource', 'title', 'status', 'activity', 'modifiedAt', 'changes', 'origin', 'movable', 'interactivity', 'workingDirectories'] as const;

function projection(state: ChatState | ChatSummary) {
  return normalized(Object.fromEntries(summaryFields.map((key) => [key, state[key]])));
}

async function assertSummary(t: TestContext, session: SessionHandle, client: Client) {
  const reader = await t.client({ initialSubscriptions: [] });
  const chat = (await reader.subscribe(session.chatUri)).state as ChatState;
  const owner = (await reader.subscribe(session.sessionUri)).state as SessionState;
  const summary = owner.chats.find((entry) => entry.resource === session.chatUri);
  assert.ok(summary, 'owning session must list its default chat');
  assert.deepEqual(projection(summary), projection(chat), 'session ChatSummary differs from ChatState');
  const live = client.state<SessionState>(session.sessionUri)?.chats.find((entry) => entry.resource === session.chatUri);
  assert.ok(live, 'existing subscriber lost the chat catalog entry');
  assert.deepEqual(projection(live), projection(summary), 'live session catalog was not synchronized with the durable chat summary');
}

async function readySession(t: TestContext) {
  const client = await t.client();
  const session = await t.session(client);
  await session.ready();
  return { client, session };
}

async function listedSummary(client: Client, uri: string): Promise<SessionSummary> {
  let cursor: string | undefined;
  const cursors = new Set<string>();
  for (let page = 0; page < 100; page++) {
    const result = await client.request<ListSessionsResult>('listSessions', { channel: 'ahp-root://', ...(cursor ? { cursor } : {}) });
    const summary = result.items.find((entry) => entry.resource === uri);
    if (summary) return summary;
    if (!result.nextCursor) break;
    assert.ok(!cursors.has(result.nextCursor), 'listSessions cursor repeated');
    cursors.add(result.nextCursor);
    cursor = result.nextCursor;
  }
  return assert.fail('initialized session was not found in listSessions');
}

async function readChange(t: TestContext, client: Client, chat: string, isRead: boolean, prerequisite = false) {
  const seq = client.dispatch(chat, { type: 'chat/isReadChanged', isRead });
  const echo = await client.waitForAction(chat, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === seq);
  if (prerequisite && echo.rejectionReason) t.skip('chat/isReadChanged was rejected; chats/is-read-changed/default-chat tests action acceptance.');
  assert.equal(echo.rejectionReason, undefined, 'chat/isReadChanged must be accepted for a known chat');
}

export const tests: TestDefinition[] = [
  {
    id: 'chats/summary/catalog-matches-created-chat', area: 'Chats & turns', feature: 'chatSummary',
    title: 'The default chat summary matches its inlined chat fields after initialization.',
    spec: '/specification/chat-channel#state',
    async run(t) {
      const { client, session } = await readySession(t);
      await assertSummary(t, session, client);
    },
  },
  {
    id: 'chats/summary/catalog-matches-completed-turn', area: 'Chats & turns', feature: 'chatSummary',
    title: 'The session catalog summary matches the chat after a model turn completes.',
    spec: '/specification/chat-channel#state', requires: [requires.model],
    async run(t) {
      const { client, session } = await readySession(t);
      await completeTurn(session, prompts.reply(t.uniqueId('summary')));
      await client.request('ping', { channel: 'ahp-root://' });
      await assertSummary(t, session, client);
    },
  },
  {
    id: 'chats/is-read-changed/default-chat', area: 'Chats & turns', feature: 'isReadChanged',
    title: 'Read changes toggle only the default chat read bit without changing the session read bit.',
    spec: '/specification/chat-channel#state',
    async run(t) {
      const { client, session } = await readySession(t);
      const beforeChat = chatState(client, session.chatUri).status;
      const beforeSession = client.state<SessionState>(session.sessionUri)!.status;
      for (const isRead of [true, false]) {
        await readChange(t, client, session.chatUri, isRead);
        const chat = await snapshotState<ChatState>(t, session.chatUri);
        const owner = await snapshotState<SessionState>(t, session.sessionUri);
        assert.equal(Boolean(chat.status & IS_READ), isRead);
        assert.equal(chat.status & ~IS_READ, beforeChat & ~IS_READ);
        assert.equal(owner.status & IS_READ, beforeSession & IS_READ);
        assert.equal(owner.chats.find((entry) => entry.resource === session.chatUri)?.status, chat.status);
      }
    },
  },
  {
    id: 'chats/is-read-changed/sibling-isolation', area: 'Chats & turns', feature: 'isReadChanged',
    title: 'Marking a non-default chat read leaves sibling and owning-session read flags unchanged.',
    spec: '/specification/chat-channel#state', requires: [requires.multipleChats],
    async run(t) {
      const { client, session } = await readySession(t);
      const chat = await createChat(t, client, session);
      const beforeSibling = chatState(client, session.chatUri).status & IS_READ;
      const beforeOwner = client.state<SessionState>(session.sessionUri)!.status & IS_READ;
      const isRead = !(chatState(client, chat).status & IS_READ);
      await readChange(t, client, chat, isRead, true);
      assert.equal(Boolean((await snapshotState<ChatState>(t, chat)).status & IS_READ), isRead);
      assert.equal((await snapshotState<ChatState>(t, session.chatUri)).status & IS_READ, beforeSibling);
      assert.equal((await snapshotState<SessionState>(t, session.sessionUri)).status & IS_READ, beforeOwner);
    },
  },
  {
    id: 'chats/is-read-changed/root-chat-projection', area: 'Chats & turns', feature: 'isReadChanged',
    title: 'An accepted chat read change synchronizes the compact root chat status projection.',
    spec: '/specification/chat-channel#state',
    async run(t) {
      const { client, session } = await readySession(t);
      const isRead = !(chatState(client, session.chatUri).status & IS_READ);
      const before = client.received.length;
      await readChange(t, client, session.chatUri, isRead, true);
      await client.waitForMessage((message) => {
        if (!client.received.slice(before).includes(message) || message.method !== 'root/sessionSummaryChanged') return false;
        const params = message.params as SessionSummaryChangedParams;
        const chat = params.changes?.chats?.find((entry) => entry.resource === session.chatUri);
        return params.channel === 'ahp-root://' && params.session === session.sessionUri && chat?.status !== undefined && Boolean(chat.status & IS_READ) === isRead;
      });
      const listed = await listedSummary(client, session.sessionUri);
      const status = listed.chats?.find((entry) => entry.resource === session.chatUri)?.status;
      assert.notEqual(status, undefined, 'compact chat projection must include the synchronized status');
      assert.equal(Boolean(status! & IS_READ), isRead);
    },
  },
];
