import type { ActionEnvelope, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, LooseStateAction, TestDefinition } from '../../harness/types.ts';
import { rename, sessionState } from './helpers.ts';

async function changeFlag(client: Client, uri: string, flag: 'isRead' | 'isArchived', value: boolean) {
  const action: LooseStateAction = flag === 'isRead'
    ? { type: 'session/isReadChanged', isRead: value }
    : { type: 'session/isArchivedChanged', isArchived: value };
  const clientSeq = client.dispatch(uri, action);
  const echo = await client.waitForAction(uri,
    (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
  assert.equal(echo.rejectionReason, undefined);
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/session-actions/title-changed',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'A client can change a session title.',
    spec: '/reference/session#sessiontitlechanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const title = t.uniqueId('title');
      await rename(client, session.sessionUri, title);
      assert.equal(sessionState(client, session.sessionUri).title, title);
    },
  },
  {
    id: 'sessions/session-actions/title-origin',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'The echoed rename action identifies the originating client and sequence.',
    spec: '/specification/subscriptions#action-delivery-action',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const title = t.uniqueId('title');
      const clientSeq = client.dispatch(session.sessionUri, { type: 'session/titleChanged', title } satisfies LooseStateAction);
      const echo = await client.waitForAction(session.sessionUri,
        (event) => event.action.type === 'session/titleChanged' && event.action.title === title);
      assert.equal(echo.origin?.clientId, client.clientId);
      assert.equal(echo.origin?.clientSeq, clientSeq);
    },
  },
  {
    id: 'sessions/session-actions/title-second-client',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'A second session subscriber receives a live rename action.',
    spec: '/specification/subscriptions#action-delivery-action',
    async run(t) {
      const writer = await t.client();
      const session = await t.session(writer);
      const observer = await t.client();
      await observer.subscribe(session.sessionUri);
      const title = t.uniqueId('title');
      await rename(writer, session.sessionUri, title, t);
      const event = await observer.waitForAction(session.sessionUri,
        (envelope) => envelope.action.type === 'session/titleChanged' && envelope.action.title === title);
      assert.equal(event.rejectionReason, undefined);
      assert.equal(sessionState(observer, session.sessionUri).title, title);
    },
  },
  {
    id: 'sessions/session-actions/title-mirror-consistency',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'The session mirror matches a fresh snapshot after a rename.',
    spec: '/reference/session#sessiontitlechanged',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await rename(client, session.sessionUri, t.uniqueId('title'), t);
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
  {
    id: 'sessions/session-actions/second-client-mirror-consistency',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'A second subscriber session mirror matches a fresh snapshot after a model turn.',
    spec: '/specification/subscriptions#action-delivery-action',
    requires: [requires.model],
    async run(t) {
      const writer = await t.client();
      const session = await t.session(writer);
      const observer = await t.client();
      await observer.subscribe(session.sessionUri);
      await observer.subscribe(session.chatUri);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      await observer.waitForAction(session.chatUri,
        (event) => event.action.type === 'chat/turnComplete' && event.action.turnId === turnId);
      await observer.request('ping', { channel: 'ahp-root://' });
      await observer.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
  {
    id: 'sessions/session-actions/unknown-session-no-echo',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'An action targeting a nonexistent session is silently ignored.',
    spec: '/specification/session-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const uri = `ahp-session:/${t.uniqueId('missing')}`;
      const clientSeq = client.dispatch(uri, { type: 'session/titleChanged', title: t.uniqueId('title') } satisfies LooseStateAction);
      await client.expectNoNotification<ActionEnvelope>('action', (event) =>
        event.channel === uri || (event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq), 400);
    },
  },
  {
    id: 'sessions/session-actions/invalid-default-chat-rejected',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'Selecting a chat absent from the session catalog is rejected.',
    spec: '/specification/session-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const clientSeq = client.dispatch(session.sessionUri, {
        type: 'session/defaultChatChanged', defaultChat: `ahp-chat:/${t.uniqueId('missing')}`,
      } satisfies LooseStateAction);
      const echo = await client.waitForAction(session.sessionUri,
        (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
      assert.notEqual(echo.rejectionReason, undefined);
    },
  },
  {
    id: 'sessions/session-actions/rejected-default-chat-preserves-state',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'A rejected default-chat change leaves the designated default unchanged.',
    spec: '/specification/session-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const before = sessionState(client, session.sessionUri).defaultChat;
      const clientSeq = client.dispatch(session.sessionUri, {
        type: 'session/defaultChatChanged', defaultChat: `ahp-chat:/${t.uniqueId('missing')}`,
      } satisfies LooseStateAction);
      const echo = await client.waitForAction(session.sessionUri,
        (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
      assert.notEqual(echo.rejectionReason, undefined);
      const observer = await t.client();
      const snapshot = await observer.subscribe(session.sessionUri);
      assert.equal((snapshot.state as SessionState).defaultChat, before);
    },
  },
  {
    id: 'sessions/session-actions/rejection-mirror-consistency',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'The session mirror matches a fresh snapshot after rejecting an invalid default chat.',
    spec: '/specification/session-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const clientSeq = client.dispatch(session.sessionUri, {
        type: 'session/defaultChatChanged', defaultChat: `ahp-chat:/${t.uniqueId('missing')}`,
      } satisfies LooseStateAction);
      const echo = await client.waitForAction(session.sessionUri,
        (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
      assert.notEqual(echo.rejectionReason, undefined);
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
  ...([
    { id: 'mark-read', flag: 'isRead', bit: 1 << 5, value: true, title: 'A client can mark a session as read.', anchor: 'sessionisreadchanged' },
    { id: 'mark-unread', flag: 'isRead', bit: 1 << 5, value: false, title: 'A client can mark a session as unread.', anchor: 'sessionisreadchanged' },
    { id: 'archive', flag: 'isArchived', bit: 1 << 6, value: true, title: 'A client can archive a session.', anchor: 'sessionisarchivedchanged' },
    { id: 'unarchive', flag: 'isArchived', bit: 1 << 6, value: false, title: 'A client can unarchive a session.', anchor: 'sessionisarchivedchanged' },
  ] as const).map((test): TestDefinition => ({
    id: `sessions/session-actions/${test.id}`,
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: test.title,
    spec: `/reference/session#${test.anchor}`,
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await changeFlag(client, session.sessionUri, test.flag, !test.value);
      await changeFlag(client, session.sessionUri, test.flag, test.value);
      assert.equal((sessionState(client, session.sessionUri).status & test.bit) !== 0, test.value);
    },
  })),
  {
    id: 'sessions/session-actions/read-preserves-archive',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'Changing the read flag preserves the independent archived flag.',
    spec: '/reference/session#sessionstatus',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await changeFlag(client, session.sessionUri, 'isArchived', true);
      await changeFlag(client, session.sessionUri, 'isRead', true);
      assert.notEqual(sessionState(client, session.sessionUri).status & (1 << 6), 0);
    },
  },
  {
    id: 'sessions/session-actions/archive-preserves-read',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'Changing the archived flag preserves the independent read flag.',
    spec: '/reference/session#sessionstatus',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await changeFlag(client, session.sessionUri, 'isRead', true);
      await changeFlag(client, session.sessionUri, 'isArchived', true);
      assert.notEqual(sessionState(client, session.sessionUri).status & (1 << 5), 0);
    },
  },
  {
    id: 'sessions/session-actions/status-mirror-consistency',
    area: 'Root & sessions',
    feature: 'sessionActions',
    title: 'The session mirror matches a fresh snapshot after changing status flags.',
    spec: '/reference/session#sessionstatus',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await changeFlag(client, session.sessionUri, 'isRead', true);
      await changeFlag(client, session.sessionUri, 'isArchived', true);
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  },
];
