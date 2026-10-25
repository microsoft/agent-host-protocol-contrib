import { sessionReducer } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, SessionState, SubscribeResult } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT, observe, renameForSetup, rootState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/root-state/actions-require-subscription',
    area: 'Root & sessions', feature: 'rootState',
    title: 'Root actions are not delivered to a connection subscribed only to a session.',
    spec: '/specification/subscriptions#action-delivery-action',
    async run(t) {
      const owner = await t.client();
      if (rootState(owner).activeSessions === undefined) t.skip('The host exposes no active-session count to trigger a root action.');
      const session = await t.session(owner);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [session.sessionUri] });
      const fromSeq = owner.lastServerSeq;
      const created = await t.session(owner);
      await created.ready();
      await observe(owner);
      if (!owner.actions(ROOT).some((event) => event.serverSeq > fromSeq && event.action.type === 'root/activeSessionsChanged')) {
        t.skip('The root control subscriber observed no count action; count emission has separate positive coverage.');
      }
      await observer.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === ROOT, 350);
      assert.equal(observer.received.some((message) => message.method === 'action'
        && (message.params as ActionEnvelope).channel === ROOT), false);
    },
  },
  {
    id: 'sessions/subscribe/snapshot-live-boundary-has-no-gap',
    area: 'Root & sessions', feature: 'subscribe',
    title: 'A snapshot and its subsequent actions retain mutations racing with subscription.',
    spec: '/reference/common#snapshot',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [] });
      for (let attempt = 0; attempt < 4; attempt++) {
        if (attempt > 0) {
          await observer.unsubscribe(session.sessionUri);
          await observer.request('ping', { channel: ROOT });
        }
        const start = observer.received.length;
        const title = t.uniqueId('subscribe-race');
        const [result] = await Promise.all([
          observer.request<SubscribeResult>('subscribe', { channel: session.sessionUri }),
          renameForSetup(t, owner, session.sessionUri, title),
        ]);
        assert.ok(result.snapshot, 'a session subscription must return a snapshot');
        assert.equal(result.snapshot.resource, session.sessionUri);
        await observer.waitForState<SessionState>(session.sessionUri, (state) => state.title === title);
        let state = result.snapshot.state as SessionState;
        const actions = observer.received.slice(start).flatMap((message) => {
          if (message.method !== 'action' || message.id !== undefined) return [];
          const envelope = message.params as ActionEnvelope;
          return envelope.channel === session.sessionUri && envelope.serverSeq > result.snapshot!.fromSeq ? [envelope] : [];
        });
        for (const envelope of actions) {
          if (envelope.rejectionReason === undefined) {
            state = sessionReducer(state, envelope.action as Parameters<typeof sessionReducer>[1]);
          }
        }
        assert.equal(state.title, title, 'the accepted racing rename must appear in the snapshot or post-snapshot actions');
      }
    },
  },
];
