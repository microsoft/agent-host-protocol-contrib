import type { ActionEnvelope, ChatState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, chatState, completeTurn, normalized, pendingKind, slowActiveTurn, snapshotState, userMessage } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'chats/reasoning/create-before-append', area: 'Chats & turns', feature: 'reasoning',
    title: 'Reasoning deltas follow creation of their target reasoning part and retain the accumulated text.',
    spec: '/reference/chat#chatreasoningaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await completeTurn(session, prompts.reply(t.uniqueId('reasoning-probe')));
      const events = client.actions(session.chatUri).filter((e) => !e.rejectionReason && 'turnId' in e.action && e.action.turnId === turn.id);
      if (!events.some((e) => (e.action.type === 'chat/responsePart' && e.action.part.kind === 'reasoning') || e.action.type === 'chat/reasoning')) {
        t.skip('This turn emitted no optional reasoning parts or reasoning deltas.');
      }
      const parts = new Map<string, { kind: string; content: string }>();
      for (const { action } of events) {
        if (action.type === 'chat/responsePart' && (action.part.kind === 'reasoning' || action.part.kind === 'markdown')) {
          parts.set(action.part.id, { kind: action.part.kind, content: action.part.content });
        }
        if (action.type === 'chat/reasoning') {
          const part = parts.get(action.partId);
          assert.ok(part, 'reasoning delta preceded response-part creation');
          assert.equal(part.kind, 'reasoning', 'reasoning delta targeted a non-reasoning part');
          part.content += action.content;
        }
      }
      for (const [id, expected] of parts) {
        if (expected.kind !== 'reasoning') continue;
        const stored = turn.responseParts.find((part) => part.kind === 'reasoning' && part.id === id);
        assert.ok(stored?.kind === 'reasoning', 'reasoning response part was not retained');
        assert.equal(stored.content, expected.content);
      }
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  },
  {
    id: 'chats/usage/latest-report-retained', area: 'Chats & turns', feature: 'usage',
    title: 'The latest emitted usage report is retained unchanged on the completed turn.',
    spec: '/reference/chat#chatusageaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await completeTurn(session, prompts.reply(t.uniqueId('usage')));
      const reports = client.actions(session.chatUri).flatMap((e) => !e.rejectionReason && e.action.type === 'chat/usage' && e.action.turnId === turn.id ? [e.action.usage] : []);
      if (!reports.length) t.skip('The provider emitted no optional chat/usage report for this turn.');
      assert.deepEqual(normalized(turn.usage), normalized(reports.at(-1)));
      const fresh = await snapshotState<ChatState>(t, session.chatUri);
      assert.deepEqual(normalized(fresh.turns.find((entry) => entry.id === turn.id)?.usage), normalized(reports.at(-1)));
    },
  },
  {
    id: 'chats/truncated/omitted-cutoff-clears-history', area: 'Chats & turns', feature: 'truncated',
    title: 'Truncating without a cutoff removes all retained turns and clears any older-history cursor.',
    spec: '/reference/chat#chattruncatedaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await completeTurn(session, prompts.reply(t.uniqueId('removed-history')));
      await accepted(client, session.chatUri, { type: 'chat/truncated' });
      const state = await snapshotState<ChatState>(t, session.chatUri);
      assert.deepEqual(state.turns, []);
      assert.equal(state.turnsNextCursor, undefined);
      assert.equal(state.activeTurn, undefined);
      assert.equal(state.status & ((1 << 3) | (1 << 4)), 0, 'empty chat must not remain in progress or input-needed');
      assert.ok(state.status & 1, 'empty chat must be idle');
    },
  },
  {
    id: 'chats/truncated/drops-active-turn', area: 'Chats & turns', feature: 'truncated',
    title: 'Truncation drops the active turn and prevents its later streaming or completion from restoring it.',
    spec: '/reference/chat#chattruncatedaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turnId = await slowActiveTurn(t, client, session);
      const echo = await accepted(client, session.chatUri, { type: 'chat/truncated' });
      assert.equal(chatState(client, session.chatUri).activeTurn, undefined);
      await client.expectNoNotification<ActionEnvelope>('action', (e) => e.channel === session.chatUri && e.serverSeq > echo.serverSeq && !e.rejectionReason &&
        'turnId' in e.action && e.action.turnId === turnId && e.action.type !== 'chat/turnCancelled', 400);
      const state = await snapshotState<ChatState>(t, session.chatUri);
      assert.equal(state.activeTurn, undefined);
      assert.deepEqual(state.turns, []);
      assert.ok(state.status & 1, 'truncation must restore idle status');
    },
  },
  {
    id: 'chats/queued/consume-head-after-active-turn', area: 'Chats & turns', feature: 'pendingMessageSet',
    title: 'SHOULD: Completing an active turn consumes the reordered queue head by removing it before starting its turn.',
    spec: '/specification/chat-channel#queued-messages', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const activeId = await slowActiveTurn(t, client, session);
      const entries = [t.uniqueId('first'), t.uniqueId('second')].map((id) => ({ id, message: userMessage(prompts.reply(id)) }));
      for (const entry of entries) await accepted(client, session.chatUri, {
        type: 'chat/pendingMessageSet', kind: pendingKind('queued'), ...entry,
      });
      const order = [entries[1], entries[0]];
      await accepted(client, session.chatUri, { type: 'chat/queuedMessagesReordered', order: order.map((entry) => entry.id) });
      if (chatState(client, session.chatUri).activeTurn?.id !== activeId) t.skip('The setup turn completed before the queue was prepared.');
      assert.deepEqual(chatState(client, session.chatUri).queuedMessages?.map((entry) => entry.id), order.map((entry) => entry.id));
      await session.waitForTurnEnd(activeId);
      const starts: ActionEnvelope[] = [];
      for (const entry of order) {
        const start = await client.waitForAction(session.chatUri, (e) => e.action.type === 'chat/turnStarted' && e.action.queuedMessageId === entry.id && !e.rejectionReason);
        assert.ok(start.action.type === 'chat/turnStarted');
        const removal = client.actions(session.chatUri).find((e) => e.action.type === 'chat/pendingMessageRemoved' && e.action.kind === 'queued' && e.action.id === entry.id && !e.rejectionReason);
        assert.ok(removal, 'consumption must emit pendingMessageRemoved');
        assert.ok(removal.serverSeq < start.serverSeq, 'queue removal must precede the resulting turn start');
        assert.equal(start.action.message.text, entry.message.text);
        starts.push(start);
        await session.waitForTurnEnd(start.action.turnId);
      }
      assert.ok(starts[0].serverSeq < starts[1].serverSeq, 'queue consumption did not follow the reordered head');
    },
  },
];
