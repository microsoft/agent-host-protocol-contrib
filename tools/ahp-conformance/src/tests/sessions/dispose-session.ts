import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { SessionRemovedParams, SessionState } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, catalogue, createSession } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/dispose-session/null-result',
    area: 'Root & sessions',
    feature: 'disposeSession',
    title: 'Disposing a session returns null on success.',
    spec: '/reference/session#disposesession',
    async run(t) {
      const client = await t.client();
      const session = await createSession(t, client);
      assert.equal(await session.dispose(), null);
    },
  },
  {
    id: 'sessions/dispose-session/session-removed',
    area: 'Root & sessions',
    feature: 'disposeSession',
    title: 'Disposing a session emits a session-removed notification on root.',
    spec: '/reference/root#rootsessionremoved',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await session.dispose();
      const event = await client.waitForNotification<SessionRemovedParams>('root/sessionRemoved',
        (params) => params.session === session.sessionUri);
      assert.equal(event.channel, ROOT);
    },
  },
  {
    id: 'sessions/dispose-session/absent-from-catalog',
    area: 'Root & sessions',
    feature: 'disposeSession',
    title: 'A disposed session is absent from a fresh session catalog page.',
    spec: '/specification/session-channel#disposal',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await session.dispose();
      assert.ok(!(await catalogue(client)).items.some((entry) => entry.resource === session.sessionUri),
        'a fresh listSessions page must not include the disposed session');
    },
  },
  {
    id: 'sessions/dispose-session/subscribe-fails',
    area: 'Root & sessions',
    feature: 'disposeSession',
    title: 'Subscribing to a disposed session returns SessionNotFound.',
    spec: '/reference/error-codes#ahp-application-codes',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.dispose();
      const observer = await t.client();
      await expectRpcError(observer.subscribe(session.sessionUri), AhpErrorCodes.SessionNotFound);
    },
  },
  {
    id: 'sessions/dispose-session/emits-chat-removed',
    area: 'Root & sessions', feature: 'disposeSession',
    title: 'Session disposal emits a chat-removed action for every cataloged chat.',
    spec: '/specification/chat-channel#disposal',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [] });
      const before = (await observer.subscribe(session.sessionUri)).state as SessionState;
      if (!before.chats.length) t.skip('The prerequisite session catalog contains no chats to remove.');
      const after = observer.lastServerSeq;
      await session.dispose();
      for (const chat of before.chats) {
        await observer.waitForAction(session.sessionUri, (event) => event.serverSeq > after &&
          event.rejectionReason === undefined && event.action.type === 'session/chatRemoved' &&
          event.action.chat === chat.resource, { timeoutMs: 5_000 });
      }
    },
  },
  {
    id: 'sessions/dispose-session/cascades-to-chat-resources',
    area: 'Root & sessions', feature: 'disposeSession',
    title: 'All cataloged chats become unsubscribable when their owning session is disposed.',
    spec: '/specification/chat-channel#disposal',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const before = (await client.subscribe(session.sessionUri)).state as SessionState;
      await session.dispose();
      const observer = await t.client({ initialSubscriptions: [] });
      const chats = new Set([session.chatUri, ...before.chats.map((chat) => chat.resource)]);
      for (const chat of chats) await assert.rejects(observer.subscribe(chat), RpcError);
    },
  },
];
