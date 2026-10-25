import type { ActionEnvelope, ChatState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import {
  assertSnapshot,
  missedChatActions,
  reconnect,
  renameForSetup,
  requireReplay,
  ROOT,
} from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'transport/reconnect/snapshot-includes-all-live-subscriptions',
    area: 'Transport & lifecycle', feature: 'reconnect',
    title: 'Snapshot recovery includes every requested live stateful subscription.',
    spec: '/reference/common#reconnectsnapshotresult',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      await session.ready();
      const subscriptions = [ROOT, session.sessionUri, session.chatUri];
      const observer = await t.client({ initialSubscriptions: subscriptions });
      const lastSeenServerSeq = observer.lastServerSeq;
      await observer.close();
      for (let index = 0; index < 32; index++) {
        await renameForSetup(t, owner, session.sessionUri, t.uniqueId('snapshot-gap'));
      }
      const { result } = await reconnect(t, observer.clientId, lastSeenServerSeq, subscriptions);
      if (result.type !== 'snapshot') return t.skip('The bounded mutation gap is still replayable; AHP specifies no replay buffer size.');
      for (const uri of subscriptions) {
        assertSnapshot(result.snapshots.find((entry) => entry.resource === uri), uri);
      }
    },
  },
  {
    id: 'transport/reconnect/disposed-channel-excluded-from-actions',
    area: 'Transport & lifecycle', feature: 'reconnect',
    title: 'Replay recovery excludes historical actions for a disposed subscription.',
    spec: '/reference/common#reconnectreplayresult',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [ROOT, session.sessionUri] });
      const lastSeenServerSeq = observer.lastServerSeq;
      await observer.close();
      await renameForSetup(t, owner, session.sessionUri, t.uniqueId('disposed-history'));
      await session.dispose();
      const { result } = await reconnect(t, observer.clientId, lastSeenServerSeq, [ROOT, session.sessionUri]);
      const replay = requireReplay(t, result);
      assert.ok(replay.actions.every((action) => action.channel !== session.sessionUri),
        'a disposed subscription cannot contribute replay actions');
    },
  },
  {
    id: 'transport/reconnect/replay-complete',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Reconnect returns every missed subscribed action in its replay response.',
    spec: '/specification/lifecycle#reconnection',
    requires: [requires.model],
    async run(t) {
      const { result, expected, finalSeq, session } = await missedChatActions(t);
      const replay = requireReplay(t, result);
      const recovered = replay.actions.filter((action) => action.channel === session.chatUri && action.serverSeq <= finalSeq);
      assert.deepEqual(recovered, expected);
    },
  },
  {
    id: 'transport/reconnect/replay-sequence-order',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Reconnect replay actions preserve the server sequence order.',
    spec: '/specification/lifecycle#reconnection',
    requires: [requires.model],
    async run(t) {
      const { result } = await missedChatActions(t);
      const replay = requireReplay(t, result);
      assert.ok(replay.actions.length >= 2, 'expected the missed turn actions in the replay response');
      for (let index = 1; index < replay.actions.length; index++) {
        assert.ok(replay.actions[index]!.serverSeq > replay.actions[index - 1]!.serverSeq);
      }
    },
  },
  {
    id: 'transport/reconnect/replay-excludes-seen-actions',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Reconnect replays only actions newer than lastSeenServerSeq.',
    spec: '/reference/common#reconnect',
    requires: [requires.model],
    async run(t) {
      const { result, lastSeenServerSeq } = await missedChatActions(t);
      const replay = requireReplay(t, result);
      for (const action of replay.actions) assert.ok(action.serverSeq > lastSeenServerSeq);
    },
  },
  {
    id: 'transport/reconnect/replay-only-requested-channels',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Reconnect replay excludes actions for channels not requested as subscriptions.',
    spec: '/reference/common#reconnect',
    requires: [requires.model],
    async run(t) {
      const { result, session } = await missedChatActions(t);
      const replay = requireReplay(t, result);
      for (const action of replay.actions) assert.equal(action.channel, session.chatUri);
    },
  },
  {
    id: 'transport/reconnect/replay-in-response-not-notifications',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Ordinary replay data is returned in the response rather than separate action notifications.',
    spec: '/specification/lifecycle#reconnection',
    requires: [requires.model],
    async run(t) {
      const { result, connection, lastSeenServerSeq, finalSeq, session } = await missedChatActions(t);
      requireReplay(t, result);
      await connection.expectNoNotification<ActionEnvelope>('action', (action) =>
        action.channel === session.chatUri && action.serverSeq > lastSeenServerSeq && action.serverSeq <= finalSeq, 350);
    },
  },
  {
    id: 'transport/reconnect/live-subscription-restored',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'A resumed ordinary subscription receives new live actions after reconnect.',
    spec: '/specification/lifecycle#reconnection',
    requires: [requires.model],
    async run(t) {
      const { connection, session, finalSeq } = await missedChatActions(t);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('after-reconnect')));
      const action = await connection.waitForNotification<ActionEnvelope>('action', (envelope) =>
        envelope.channel === session.chatUri && envelope.action.type === 'chat/turnStarted' &&
        envelope.action.turnId === turnId);
      assert.ok(action.serverSeq > finalSeq);
      await session.waitForTurnEnd(turnId);
    },
  },
  {
    id: 'transport/reconnect/snapshot-fallback-state',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Snapshot recovery returns current state for the resumed ordinary subscription.',
    spec: '/specification/lifecycle#reconnection',
    requires: [requires.model],
    async run(t) {
      const { result, session, turnId, finalSeq } = await missedChatActions(t);
      if (result.type !== 'snapshot') return t.skip('This gap is still replayable; the protocol does not specify a replay buffer size.');
      const snapshot = result.snapshots.find((entry) => entry.resource === session.chatUri);
      if (!snapshot) t.skip('The host did not resume this chat, possibly because access changed.');
      assertSnapshot(snapshot, session.chatUri);
      assert.ok((snapshot.state as ChatState).turns.some((turn) => turn.id === turnId));
      assert.ok(snapshot.fromSeq >= finalSeq);
    },
  },
  {
    id: 'transport/reconnect/snapshot-only-requested-channels',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Snapshot recovery excludes channels not requested as subscriptions.',
    spec: '/reference/common#reconnect',
    requires: [requires.model],
    async run(t) {
      const { result, session } = await missedChatActions(t);
      if (result.type !== 'snapshot') return t.skip('The host selected replay recovery for this gap.');
      for (const snapshot of result.snapshots) assert.equal(snapshot.resource, session.chatUri);
    },
  },
  {
    id: 'transport/reconnect/disposed-subscription-missing',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Replay recovery reports a disposed session in the missing subscription list.',
    spec: '/reference/common#reconnect',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      const observer = await t.client({ initialSubscriptions: [ROOT, session.sessionUri] });
      const lastSeenServerSeq = observer.lastServerSeq;
      await observer.close();
      await session.dispose();
      const { result } = await reconnect(t, observer.clientId, lastSeenServerSeq, [ROOT, session.sessionUri]);
      const replay = requireReplay(t, result);
      assert.ok(Array.isArray(replay.missing));
      assert.ok(replay.missing.includes(session.sessionUri));
    },
  },
  {
    id: 'transport/reconnect/protocol-notifications-not-replayed',
    area: 'Transport & lifecycle',
    feature: 'reconnect',
    title: 'Ordinary reconnect does not replay protocol notifications missed while disconnected.',
    spec: '/specification/subscriptions#protocol-notifications',
    async run(t) {
      const writer = await t.client();
      const observer = await t.client();
      const lastSeenServerSeq = observer.lastServerSeq;
      await observer.close();
      const session = await t.session(writer);
      await session.ready();
      await writer.waitForNotification<{ summary: { resource: string } }>('root/sessionAdded',
        (params) => params.summary.resource === session.sessionUri);
      const { connection } = await reconnect(t, observer.clientId, lastSeenServerSeq, [ROOT]);
      await connection.expectNoNotification<{ summary: { resource: string } }>('root/sessionAdded',
        (params) => params.summary.resource === session.sessionUri, 400);
    },
  },
];
