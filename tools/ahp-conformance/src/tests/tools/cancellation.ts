import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import {
  cancelledWriteTurn, fileExists, requireConfirmation, runToolTurn,
  toolCalls, toolTest, writeTarget,
} from './helpers.ts';

export const tests: TestDefinition[] = [
  toolTest({
    id: 'tools/cancellation/awaiting-confirmation-turn-cancelled', feature: 'turnCancelled',
    title: 'Cancelling a turn awaiting tool confirmation records the turn as cancelled.',
    spec: '/reference/chat#chatturncancelledaction',
    async run(t) {
      const { turn } = await cancelledWriteTurn(t);
      assert.equal(turn.state, 'cancelled');
    },
  }),
  toolTest({
    id: 'tools/cancellation/awaiting-confirmation-tool-skipped', feature: 'turnCancelled',
    title: 'Cancelling a turn force-cancels its pending tool call with reason skipped.',
    spec: '/guide/state-model#editable-parameters',
    async run(t) {
      const { turn, decisions } = await cancelledWriteTurn(t);
      const pending = decisions.find(({ decision }) => decision === 'cancel');
      assert(pending);
      const call = toolCalls(turn).find((call) => call.toolCallId === pending.toolCall.toolCallId);
      assert(call?.status === 'cancelled', 'pending tool call was not cancelled');
      assert.equal(call.reason, 'skipped');
    },
  }),
  toolTest({
    id: 'tools/cancellation/awaiting-confirmation-prevents-write', feature: 'turnCancelled',
    title: 'Cancelling a file-write turn before confirmation prevents file creation.',
    spec: '/reference/chat#chatturncancelledaction',
    async run(t) {
      const target = writeTarget(t);
      const client = await t.client();
      const session = await t.session(client);
      const { decisions } = await runToolTurn(client, session, target.prompt, {
        async confirmation() {
          if (await fileExists(target.filePath)) t.skip('host requested confirmation only after the file was created');
          return 'cancel' as const;
        },
      });
      requireConfirmation(t, decisions);
      assert.equal(await fileExists(target.filePath), false, 'file was created after cancellation');
    },
  }),
  toolTest({
    id: 'tools/cancellation/mirror-matches-snapshot', feature: 'turnCancelled',
    title: 'The chat mirror matches a fresh snapshot after cancelling a pending tool turn.',
    spec: '/guide/actions#reducers',
    async run(t) {
      const { client, session } = await cancelledWriteTurn(t);
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  }),
];
