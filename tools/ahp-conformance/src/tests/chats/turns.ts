import type { ActionEnvelope, ChatState, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, cancelledTurn, chatState, completeTurn, dispatch, userMessage } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'chats/turn-started/echo-origin',
    area: 'Chats & turns', feature: 'turnStarted',
    title: 'A started turn is echoed with its dispatching client and sequence.',
    spec: '/guide/actions#action-envelope', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turnId = t.uniqueId('turn');
      const clientSeq = dispatch(client, session.chatUri, {
        type: 'chat/turnStarted', turnId, startedAt: new Date().toISOString(),
        message: userMessage(prompts.reply(t.uniqueId('reply'))),
      });
      const echo = await client.waitForAction(session.chatUri, (e) =>
        e.action.type === 'chat/turnStarted' && e.action.turnId === turnId);
      assert.deepEqual(echo.origin, { clientId: client.clientId, clientSeq });
      assert.equal(echo.rejectionReason, undefined);
      await session.waitForTurnEnd(turnId);
    },
  },
  {
    id: 'chats/delta/response-part-before-delta',
    area: 'Chats & turns', feature: 'delta',
    title: 'Every text delta follows creation of its target markdown response part.',
    spec: '/reference/chat#chatdeltaaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turn = await completeTurn(session, prompts.reply(t.uniqueId('reply'), { chunks: 4 }));
      const parts = new Set<string>();
      for (const { action } of client.actions(session.chatUri)) {
        if (!('turnId' in action) || action.turnId !== turn.id) continue;
        if (action.type === 'chat/responsePart' && action.part.kind === 'markdown') parts.add(action.part.id);
        if (action.type === 'chat/delta') assert.ok(parts.has(action.partId), `delta precedes part ${action.partId}`);
      }
    },
  },
  {
    id: 'chats/delta/concatenated-reply',
    area: 'Chats & turns', feature: 'delta',
    title: 'Response parts and their deltas reconstruct the requested reply.',
    spec: '/reference/chat#chatdeltaaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const text = `${t.uniqueId('reply')} alpha beta gamma delta`;
      const turn = await completeTurn(session, prompts.reply(text, { chunks: 5, waitMs: 25 }));
      const parts = new Map<string, string>();
      for (const { action } of client.actions(session.chatUri)) {
        if (!('turnId' in action) || action.turnId !== turn.id) continue;
        if (action.type === 'chat/responsePart' && action.part.kind === 'markdown') parts.set(action.part.id, action.part.content);
        if (action.type === 'chat/delta') parts.set(action.partId, (parts.get(action.partId) ?? '') + action.content);
      }
      assert.ok([...parts.values()].join('').trim().includes(text));
    },
  },
  {
    id: 'chats/turn-complete/terminal-state',
    area: 'Chats & turns', feature: 'turnComplete',
    title: 'A completed turn is retained with the complete terminal state.',
    spec: '/reference/chat#chatturncompleteaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')));
      await client.waitForAction(session.chatUri, (e) => e.action.type === 'chat/turnComplete' && e.action.turnId === turnId);
      const turn = await session.waitForTurnEnd<Turn>(turnId);
      assert.equal(turn.state, 'complete');
    },
  },
  {
    id: 'chats/turn-complete/mirror-matches-snapshot',
    area: 'Chats & turns', feature: 'turnComplete',
    title: 'The completed-turn mirror matches a fresh chat snapshot.',
    spec: '/specification/chat-channel#active-chat', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await completeTurn(session, prompts.reply(t.uniqueId('reply'), { chunks: 4 }));
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  },
  {
    id: 'chats/error/upstream-failure-state',
    area: 'Chats & turns', feature: 'error',
    title: 'An upstream failure ends the turn in the error state.',
    spec: '/specification/chat-channel#error-recovery', requires: [requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const id = session.startTurn(prompts.upstreamError(400, t.uniqueId('failure')));
      const turn = await session.waitForTurnEnd<Turn>(id);
      assert.equal(turn.state, 'error');
    },
  },
  {
    id: 'chats/error/error-part-via-error-action',
    area: 'Chats & turns', feature: 'error',
    title: 'An upstream failure appends its error response part through chat/error.',
    spec: '/specification/chat-channel#error-recovery', requires: [requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const id = session.startTurn(prompts.upstreamError(400, t.uniqueId('failure')));
      const turn = await session.waitForTurnEnd<Turn>(id);
      const error = client.actions(session.chatUri).find((e) => e.action.type === 'chat/error' && e.action.turnId === id);
      assert.ok(error && error.action.type === 'chat/error', 'missing chat/error');
      assert.equal(error.action.part.kind, 'error');
      assert.deepEqual(turn.responseParts.at(-1), error.action.part);
      assert.ok(!client.actions(session.chatUri).some((e) => e.action.type === 'chat/responsePart' &&
        e.action.turnId === id && e.action.part.kind === 'error'));
    },
  },
  {
    id: 'chats/turn-cancelled/terminal-state',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'Cancelling an active turn retains it with the cancelled terminal state.',
    spec: '/reference/chat#chatturncancelledaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const { turn } = await cancelledTurn(t, client, session);
      assert.equal(turn.state, 'cancelled');
    },
  },
  {
    id: 'chats/turn-cancelled/no-later-deltas',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'A cancelled turn emits no text deltas after cancellation is accepted.',
    spec: '/reference/chat#chatturncancelledaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const { turnId, envelope } = await cancelledTurn(t, client, session);
      await client.expectNoNotification<ActionEnvelope>('action', (e) => e.channel === session.chatUri &&
        e.serverSeq > envelope.serverSeq && e.action.type === 'chat/delta' && e.action.turnId === turnId, 500);
    },
  },
  {
    id: 'chats/turn-cancelled/no-later-completion',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'A cancelled turn emits no completion after cancellation is accepted.',
    spec: '/reference/chat#chatturncancelledaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const { turnId, envelope } = await cancelledTurn(t, client, session);
      await client.expectNoNotification<ActionEnvelope>('action', (e) => e.channel === session.chatUri &&
        e.serverSeq > envelope.serverSeq && e.action.type === 'chat/turnComplete' && e.action.turnId === turnId, 500);
    },
  },
  {
    id: 'chats/turn-cancelled/snapshot-terminal-state',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'A fresh snapshot records the cancelled turn as cancelled.',
    spec: '/reference/chat#chatturncancelledaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const { turnId } = await cancelledTurn(t, client, session);
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = await observer.subscribe(session.chatUri);
      assert.equal((snapshot.state as ChatState).turns.find((turn) => turn.id === turnId)?.state, 'cancelled');
    },
  },
  {
    id: 'chats/turn-cancelled/mirror-matches-snapshot',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'The cancelled-turn mirror matches a fresh chat snapshot.',
    spec: '/reference/chat#chatturncancelledaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await cancelledTurn(t, client, session);
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  },
  {
    id: 'chats/turn-cancelled/idle-rejected',
    area: 'Chats & turns', feature: 'turnCancelled',
    title: 'Cancelling a chat with no active turn is rejected.',
    spec: '/specification/chat-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const clientSeq = dispatch(client, session.chatUri, { type: 'chat/turnCancelled', turnId: t.uniqueId('missing'), duration: 0 });
      const echo = await client.waitForAction(session.chatUri, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq);
      assert.ok(echo.rejectionReason);
    },
  },
  {
    id: 'chats/turn-resume/non-errored-turn-rejected',
    area: 'Chats & turns', feature: 'turnResume',
    title: 'Resuming a turn that is not the latest resumable error is rejected.',
    spec: '/specification/chat-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const clientSeq = dispatch(client, session.chatUri, { type: 'chat/turnResume', turnId: t.uniqueId('missing') });
      const echo = await client.waitForAction(session.chatUri, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq);
      assert.ok(echo.rejectionReason);
    },
  },
  {
    id: 'chats/history/previous-response-context',
    area: 'Chats & turns', feature: 'history',
    title: 'A second turn can repeat the preceding assistant response.',
    spec: '/specification/chat-channel#active-chat', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('previous');
      await completeTurn(session, prompts.reply(phrase));
      const turn = await completeTurn(session, prompts.repeatPrevious());
      assert.ok(session.responseText(turn.id).trim().includes(phrase));
    },
  },
  {
    id: 'chats/history/remembered-user-context',
    area: 'Chats & turns', feature: 'history',
    title: 'A later turn receives the earlier user message as context.',
    spec: '/specification/chat-channel#active-chat', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const phrase = t.uniqueId('remember');
      await completeTurn(session, prompts.remember(phrase));
      const turn = await completeTurn(session, prompts.recall());
      assert.ok(session.responseText(turn.id).trim().includes(phrase));
    },
  },
  {
    id: 'chats/truncated/retains-cutoff-turn',
    area: 'Chats & turns', feature: 'truncated',
    title: 'Truncating history retains the cutoff turn and removes later turns.',
    spec: '/reference/chat#chattruncatedaction', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const first = await completeTurn(session, prompts.reply(t.uniqueId('first')));
      await completeTurn(session, prompts.reply(t.uniqueId('second')));
      await accepted(client, session.chatUri, { type: 'chat/truncated', turnId: first.id });
      assert.deepEqual(chatState(client, session.chatUri).turns.map((turn) => turn.id), [first.id]);
    },
  },
];
