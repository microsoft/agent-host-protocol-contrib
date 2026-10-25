import { readFile } from 'node:fs/promises';
import type { ActionEnvelope } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { LooseStateAction, TestDefinition } from '../../harness/types.ts';
import {
  dispatchAndWait, fileExists, readTurn, requireConfirmation, runToolTurn,
  toolCalls, toolTest, writeTarget, writeTurn,
} from './helpers.ts';

export const tests: TestDefinition[] = [
  toolTest({
    id: 'tools/confirmations/approval-allows-execution', feature: 'toolCallConfirmed',
    title: 'Approving a requested tool confirmation allows the file write to execute.',
    spec: '/reference/chat#chattoolcallapprovedaction',
    async run(t) {
      const { decisions, filePath, contents } = await writeTurn(t);
      requireConfirmation(t, decisions);
      assert.equal(await readFile(filePath, 'utf8'), contents);
    },
  }),
  toolTest({
    id: 'tools/confirmations/user-approval-recorded', feature: 'toolCallConfirmed',
    title: 'An approved tool call retains the user-action confirmation reason.',
    spec: '/reference/chat#toolcallconfirmationreason',
    async run(t) {
      const { decisions, turn } = await writeTurn(t);
      for (const { toolCall } of requireConfirmation(t, decisions)) {
        const call = toolCalls(turn).find((call) => call.toolCallId === toolCall.toolCallId);
        assert(call?.status === 'completed', 'approved tool call did not complete');
        assert.equal(call.confirmed, 'user-action');
      }
    },
  }),
  toolTest({
    id: 'tools/confirmations/waits-for-approval', feature: 'toolCallConfirmed',
    title: 'A tool awaiting confirmation does not complete before client approval.',
    spec: '/reference/chat#toolcallpendingconfirmationstate',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const { prompt } = writeTarget(t);
      const { decisions } = await runToolTurn(client, session, prompt, {
        async confirmation(call) {
          const after = client.lastServerSeq;
          await client.expectNoNotification<ActionEnvelope>('action', (envelope) =>
            envelope.channel === session.chatUri && envelope.serverSeq > after &&
            envelope.rejectionReason === undefined && envelope.action.type === 'chat/toolCallComplete' &&
            envelope.action.toolCallId === call.toolCallId, 400);
          return 'approve' as const;
        },
      });
      requireConfirmation(t, decisions);
    },
  }),
  toolTest({
    id: 'tools/confirmations/denial-cancels-tool', feature: 'toolCallConfirmed',
    title: 'Denying a tool confirmation records the tool as cancelled with reason denied.',
    spec: '/reference/chat#chattoolcalldeniedaction',
    async run(t) {
      const { decisions, turn } = await writeTurn(t, { confirmation: () => 'deny' });
      for (const { toolCall } of requireConfirmation(t, decisions)) {
        const call = toolCalls(turn).find((call) => call.toolCallId === toolCall.toolCallId);
        assert(call?.status === 'cancelled', 'denied tool call was not cancelled');
        assert.equal(call.reason, 'denied');
      }
    },
  }),
  toolTest({
    id: 'tools/confirmations/denial-prevents-file-creation', feature: 'toolCallConfirmed',
    title: 'Denying the file-write confirmation prevents the file from being created.',
    spec: '/guide/actions#client-dispatched-actions',
    async run(t) {
      const target = writeTarget(t);
      const client = await t.client();
      const session = await t.session(client);
      const { decisions } = await runToolTurn(client, session, target.prompt, {
        async confirmation() {
          if (await fileExists(target.filePath)) t.skip('host requested confirmation only after the file was created');
          return 'deny' as const;
        },
      });
      requireConfirmation(t, decisions);
      assert.equal(await fileExists(target.filePath), false, 'file was created despite denial');
    },
  }),
  toolTest({
    id: 'tools/confirmations/selected-option-recorded', feature: 'toolCallConfirmed',
    title: 'A selected approval option is retained on the completed tool call.',
    spec: '/guide/state-model#confirmation-options',
    async run(t) {
      const { decisions, turn } = await writeTurn(t);
      const latest = new Map(requireConfirmation(t, decisions).map((decision) => [decision.toolCall.toolCallId, decision]));
      const offered = [...latest.values()].filter(({ toolCall }) =>
        toolCall.status === 'pending-confirmation' && toolCall.options?.some((option) => option.kind === 'approve'));
      if (!offered.length) t.skip('host did not offer an approval option');
      for (const { toolCall } of offered) {
        assert(toolCall.status === 'pending-confirmation');
        const expected = toolCall.options?.find((option) => option.kind === 'approve');
        const call = toolCalls(turn).find((call) => call.toolCallId === toolCall.toolCallId);
        assert(call && 'selectedOption' in call, 'selected confirmation option was not retained');
        assert.deepEqual(call.selectedOption, expected);
      }
    },
  }),
  toolTest({
    id: 'tools/confirmations/approve-result-completes-tool', feature: 'toolCallResultConfirmed',
    title: 'Approving a pending tool result moves the tool call to completed.',
    spec: '/reference/chat#chattoolcallresultconfirmedaction',
    async run(t) {
      const { decisions, turn } = await readTurn(t);
      const results = decisions.filter(({ toolCall }) => toolCall.status === 'pending-result-confirmation');
      if (!results.length) t.skip('host did not request result confirmation');
      for (const { toolCall } of results) {
        const call = toolCalls(turn).find((call) => call.toolCallId === toolCall.toolCallId);
        assert.equal(call?.status, 'completed');
      }
    },
  }),
  toolTest({
    id: 'tools/confirmations/deny-result-cancels-tool', feature: 'toolCallResultConfirmed',
    title: 'Denying a pending tool result cancels the tool call with reason result-denied.',
    spec: '/reference/chat#chattoolcallresultconfirmedaction',
    async run(t) {
      const { decisions, turn } = await readTurn(t, { resultConfirmation: () => 'deny' });
      const results = decisions.filter(({ toolCall }) => toolCall.status === 'pending-result-confirmation');
      if (!results.length) t.skip('host did not request result confirmation');
      for (const { toolCall } of results) {
        const call = toolCalls(turn).find((call) => call.toolCallId === toolCall.toolCallId);
        assert(call?.status === 'cancelled', 'result denial did not cancel the tool call');
        assert.equal(call.reason, 'result-denied');
      }
    },
  }),
  toolTest({
    id: 'tools/confirmations/reject-completed-tool-confirmation', feature: 'toolCallConfirmed',
    title: 'The host rejects a confirmation for an already completed tool call.',
    spec: '/specification/chat-channel#server-validation-of-client-actions',
    async run(t) {
      const { client, session, turnId, turn } = await readTurn(t);
      const completed = toolCalls(turn).find((call) => call.status === 'completed');
      assert(completed, 'no completed tool call to confirm');
      const envelope = await dispatchAndWait(client, session.chatUri, {
        type: 'chat/toolCallConfirmed', turnId, toolCallId: completed.toolCallId,
        approved: true, confirmed: 'user-action',
      } as LooseStateAction);
      assert(typeof envelope.rejectionReason === 'string' && envelope.rejectionReason.length > 0, 'rejection reason was empty');
    },
  }),
];
