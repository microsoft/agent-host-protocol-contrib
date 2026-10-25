import type { SessionRemovedParams, SessionSummaryChangedParams } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { observe, renameForSetup } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/root-notifications/removed-requires-subscription',
    area: 'Root & sessions', feature: 'rootNotifications',
    title: 'Session removal notifications are not delivered without a root subscription.',
    spec: '/specification/subscriptions#protocol-notifications',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [session.sessionUri] });
      await session.dispose();
      await observe(owner);
      const emitted = owner.received.some((message) => message.method === 'root/sessionRemoved'
        && (message.params as SessionRemovedParams).session === session.sessionUri);
      if (!emitted) t.skip('The root control subscriber observed no removal event; positive removal delivery is tested separately.');
      const matches = (params: SessionRemovedParams) => params.session === session.sessionUri;
      await observer.expectNoNotification<SessionRemovedParams>('root/sessionRemoved', matches, 350);
      assert.equal(observer.received.some((message) => message.method === 'root/sessionRemoved'
        && matches(message.params as SessionRemovedParams)), false);
    },
  },
  {
    id: 'sessions/root-notifications/summary-requires-subscription',
    area: 'Root & sessions', feature: 'rootNotifications',
    title: 'Emitted summary changes are not delivered without a root subscription.',
    spec: '/specification/subscriptions#protocol-notifications',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [session.sessionUri] });
      const title = t.uniqueId('private-summary');
      await renameForSetup(t, owner, session.sessionUri, title);
      await observe(owner);
      const matches = (params: SessionSummaryChangedParams) => params.session === session.sessionUri;
      const emitted = owner.received.some((message) => message.method === 'root/sessionSummaryChanged'
        && matches(message.params as SessionSummaryChangedParams)
        && (message.params as SessionSummaryChangedParams).changes.title === title);
      if (!emitted) t.skip('The root control subscriber observed no optional summary change for the controlled rename.');
      await observer.expectNoNotification<SessionSummaryChangedParams>('root/sessionSummaryChanged', matches, 350);
      assert.equal(observer.received.some((message) => message.method === 'root/sessionSummaryChanged'
        && matches(message.params as SessionSummaryChangedParams)), false);
    },
  },
];
