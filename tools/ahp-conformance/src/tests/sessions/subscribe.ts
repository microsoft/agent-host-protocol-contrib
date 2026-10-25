import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, ChatState, RootState } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/subscribe/root-snapshot',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'Subscribing to root returns a snapshot of the root channel.',
    spec: '/specification/subscriptions#subscribe-request',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const snapshot = await client.subscribe(ROOT);
      assert.equal(snapshot.resource, ROOT);
      assert.ok(Array.isArray((snapshot.state as RootState).agents));
    },
  },
  {
    id: 'sessions/subscribe/unknown-session',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'Subscribing to an unknown session returns SessionNotFound.',
    spec: '/reference/error-codes#ahp-application-codes',
    async run(t) {
      const client = await t.client();
      await expectRpcError(client.subscribe(`ahp-session:/${t.uniqueId('missing')}`), AhpErrorCodes.SessionNotFound);
    },
  },
  {
    id: 'sessions/subscribe/invalid-channel-type',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'A subscribe request with a non-string channel receives a JSON-RPC response.',
    spec: '/reference/common#subscribe',
    async run(t) {
      const client = await t.client();
      try {
        await client.request('subscribe', { channel: 42 });
      } catch (error) {
        assert.ok(error instanceof RpcError, 'a subscribe failure must be a JSON-RPC error');
      }
    },
  },
  {
    id: 'sessions/subscribe/snapshot-sequence',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'A session subscription snapshot carries a non-negative sequence number.',
    spec: '/reference/common#snapshot',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const snapshot = await client.subscribe(session.sessionUri);
      assert.ok(Number.isInteger(snapshot.fromSeq) && snapshot.fromSeq >= 0);
    },
  },
  {
    id: 'sessions/subscribe/actions-after-snapshot',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'An action sent after a subscription snapshot has a greater server sequence.',
    spec: '/specification/subscriptions#subscribe-request',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const snapshot = await client.subscribe(session.chatUri);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      const envelope = await client.waitForAction(session.chatUri,
        (event) => event.action.type === 'chat/turnStarted' && event.action.turnId === turnId);
      await session.waitForTurnEnd(turnId);
      assert.ok(envelope.serverSeq > snapshot.fromSeq);
    },
  },
  {
    id: 'sessions/subscribe/repeated-snapshot',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'Subscribing again returns a fresh snapshot of the existing subscription.',
    spec: '/specification/subscriptions#subscribe-request',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await session.waitForTurnEnd(turnId);
      const snapshot = await client.subscribe(session.chatUri);
      assert.ok((snapshot.state as ChatState).turns.some((turn) => turn.id === turnId));
    },
  },
  {
    id: 'sessions/subscribe/single-delivery-after-resubscribe',
    area: 'Root & sessions',
    feature: 'subscribe',
    title: 'Repeated subscription does not duplicate action delivery.',
    spec: '/specification/subscriptions#subscribe-request',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await client.subscribe(session.chatUri);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      const echo = await client.waitForAction(session.chatUri,
        (event) => event.action.type === 'chat/turnStarted' && event.action.turnId === turnId);
      await session.waitForTurnEnd(turnId);
      await client.expectNoNotification<ActionEnvelope>('action', (event) =>
        event.channel === session.chatUri && event.action.type === 'chat/turnStarted'
          && event.action.turnId === turnId && event.serverSeq !== echo.serverSeq,
      400);
      const matching = client.received.filter((message) => {
        if (message.method !== 'action') return false;
        const event = message.params as ActionEnvelope;
        return event.channel === session.chatUri && event.action.type === 'chat/turnStarted' && event.action.turnId === turnId;
      });
      assert.equal(matching.length, 1);
    },
  },
];
