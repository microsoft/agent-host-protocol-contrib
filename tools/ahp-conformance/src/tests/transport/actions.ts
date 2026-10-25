import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';

export const tests: TestDefinition[] = [
  {
    id: 'transport/actions/strictly-increasing-server-sequence',
    area: 'Transport & lifecycle',
    feature: 'actions',
    title: 'Action envelope server sequences increase strictly across subscribed channels.',
    spec: '/specification/subscriptions#action-delivery-action',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const first = await t.session(client);
      const firstTurn = first.startTurn(prompts.reply(t.uniqueId('first-reply')));
      await first.waitForTurnEnd(firstTurn);
      const second = await t.session(client);
      const secondTurn = second.startTurn(prompts.reply(t.uniqueId('second-reply')));
      await second.waitForTurnEnd(secondTurn);
      const actions = client.actions();
      assert.ok(actions.length >= 2);
      for (let index = 1; index < actions.length; index++) {
        assert.ok(actions[index]!.serverSeq > actions[index - 1]!.serverSeq,
          `serverSeq did not increase: ${actions[index - 1]!.serverSeq} -> ${actions[index]!.serverSeq}`);
      }
    },
  },
  {
    id: 'transport/actions/after-snapshot-sequence',
    area: 'Transport & lifecycle',
    feature: 'actions',
    title: 'Actions delivered after a snapshot have server sequences greater than its fromSeq.',
    spec: '/specification/subscriptions#subscribe-request',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const snapshot = await client.subscribe(session.chatUri);
      const start = client.actions(session.chatUri).length;
      const turnId = session.startTurn(prompts.reply(t.uniqueId('after-snapshot')));
      await session.waitForTurnEnd(turnId);
      const actions = client.actions(session.chatUri).slice(start);
      assert.ok(actions.length > 0);
      for (const action of actions) assert.ok(action.serverSeq > snapshot.fromSeq);
    },
  },
  {
    id: 'transport/actions/mirror-matches-snapshot',
    area: 'Transport & lifecycle',
    feature: 'actions',
    title: 'The reducer mirror matches a fresh chat snapshot after a completed turn.',
    spec: '/specification/subscriptions#action-delivery-action',
    requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('mirror-reply')));
      await session.waitForTurnEnd(turnId);
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  },
];
