import type {
  SessionAddedParams,
  SessionRemovedParams,
  SessionSummaryChangedParams,
} from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, createSession, rename, rootState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/root-notifications/session-added',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Session creation notifies the creating root subscriber.',
    spec: '/reference/root#rootsessionadded',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const event = await client.waitForNotification<SessionAddedParams>('root/sessionAdded',
        (params) => params.summary.resource === session.sessionUri);
      assert.equal(event.channel, ROOT);
      assert.equal(event.summary.provider, t.profile.agent.provider);
    },
  },
  {
    id: 'sessions/root-notifications/session-added-second-client',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Session creation notifies a second root subscriber.',
    spec: '/reference/root#rootsessionadded',
    async run(t) {
      const observer = await t.client();
      const writer = await t.client();
      const session = await t.session(writer);
      await session.ready();
      const event = await observer.waitForNotification<SessionAddedParams>('root/sessionAdded',
        (params) => params.summary.resource === session.sessionUri);
      assert.equal(event.channel, ROOT);
    },
  },
  {
    id: 'sessions/root-notifications/not-sent-without-root-subscription',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Session-added events are not delivered without a root subscription.',
    spec: '/specification/subscriptions#protocol-notifications',
    async run(t) {
      const observer = await t.client({ initialSubscriptions: [] });
      const writer = await t.client();
      const session = await t.session(writer);
      await session.ready();
      await observer.expectNoNotification<SessionAddedParams>('root/sessionAdded',
        (event) => event.summary.resource === session.sessionUri, 400);
    },
  },
  {
    id: 'sessions/root-notifications/session-summary-title',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'SHOULD: Renaming a session publishes its changed title to root subscribers.',
    spec: '/reference/root#rootsessionsummarychanged',
    async run(t) {
      const observer = await t.client();
      const writer = await t.client();
      const session = await t.session(writer);
      await session.ready();
      const title = t.uniqueId('title');
      await rename(writer, session.sessionUri, title, t);
      const event = await observer.waitForNotification<SessionSummaryChangedParams>('root/sessionSummaryChanged',
        (params) => params.session === session.sessionUri && params.changes.title === title);
      assert.equal(event.channel, ROOT);
    },
  },
  {
    id: 'sessions/root-notifications/summary-omits-identity',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Emitted session-summary changes omit immutable identity fields.',
    spec: '/specification/root-channel#rootsessionsummarychanged',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      await client.request('ping', { channel: ROOT });
      const events = client.received
        .filter((message) => message.method === 'root/sessionSummaryChanged')
        .map((message) => message.params as SessionSummaryChangedParams)
        .filter((event) => event.session === session.sessionUri);
      if (events.length === 0) t.skip('No optional summary-change notification has been observed.');
      for (const event of events) {
        for (const field of ['resource', 'provider', 'createdAt']) {
          assert.ok(!Object.hasOwn(event.changes, field), `${field} must be omitted from summary changes`);
        }
      }
    },
  },
  {
    id: 'sessions/root-notifications/session-removed-second-client',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Session disposal notifies a second root subscriber.',
    spec: '/reference/root#rootsessionremoved',
    async run(t) {
      const writer = await t.client();
      const session = await t.session(writer);
      await session.ready();
      const observer = await t.client();
      await session.dispose();
      const event = await observer.waitForNotification<SessionRemovedParams>('root/sessionRemoved',
        (params) => params.session === session.sessionUri);
      assert.equal(event.channel, ROOT);
    },
  },
  {
    id: 'sessions/root-notifications/active-count-on-create',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Creating a session publishes an active-session count when the count is tracked.',
    spec: '/reference/root#rootactivesessionschanged',
    async run(t) {
      const client = await t.client();
      if (rootState(client).activeSessions === undefined) t.skip('Root state does not track activeSessions.');
      const fromSeq = client.lastServerSeq;
      await createSession(t, client);
      const envelope = await client.waitForAction(ROOT,
        (event) => event.serverSeq > fromSeq && event.action.type === 'root/activeSessionsChanged'
          && event.action.activeSessions >= 1);
      assert.ok(envelope.action.type === 'root/activeSessionsChanged');
      assert.ok(Number.isInteger(envelope.action.activeSessions));
    },
  },
  {
    id: 'sessions/root-notifications/active-count-on-dispose',
    area: 'Root & sessions',
    feature: 'rootNotifications',
    title: 'Disposing a session publishes an active-session count when the count is tracked.',
    spec: '/reference/root#rootactivesessionschanged',
    async run(t) {
      const client = await t.client();
      if (rootState(client).activeSessions === undefined) t.skip('Root state does not track activeSessions.');
      const session = await t.session(client);
      await client.request('ping', { channel: ROOT });
      const fromSeq = client.lastServerSeq;
      await session.dispose();
      const envelope = await client.waitForAction(ROOT,
        (event) => event.serverSeq > fromSeq && event.action.type === 'root/activeSessionsChanged');
      assert.ok(envelope.action.type === 'root/activeSessionsChanged');
      assert.ok(Number.isInteger(envelope.action.activeSessions) && envelope.action.activeSessions >= 0);
    },
  },
];
