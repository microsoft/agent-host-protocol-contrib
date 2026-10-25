import type { ChatState, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, TestDefinition } from '../../harness/types.ts';
import { calls, confirmationAction, heldConfirmation, jsonValue } from './coverage-helpers.ts';

async function aggregate(client: Client, session: string, chat: string, turnId: string, toolCallId: string) {
  const state = await client.waitForState<SessionState>(session, (state) => state.inputNeeded?.some((entry) =>
    entry.kind === 'toolConfirmation' && entry.chat === chat && entry.turnId === turnId &&
    entry.toolCall.toolCallId === toolCallId) === true, { timeoutMs: 5_000 });
  const entry = state.inputNeeded?.find((entry) => entry.kind === 'toolConfirmation' && entry.chat === chat &&
    entry.turnId === turnId && entry.toolCall.toolCallId === toolCallId);
  assert.ok(entry?.kind === 'toolConfirmation');
  return entry;
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/input-needed/mirrors-pending-confirmation',
    area: 'Root & sessions', feature: 'inputNeeded',
    title: 'A session-only subscriber sees a self-sufficient aggregate of a pending tool confirmation.',
    spec: '/specification/session-channel#aggregated-input-requests',
    requires: [requires.model, requires.sharedFilesystem],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const observer = await t.client({ initialSubscriptions: [] });
      await observer.subscribe(session.sessionUri);
      const pending = await heldConfirmation(t, client, session);
      const entry = await aggregate(observer, session.sessionUri, pending.chat, pending.turnId, pending.call.toolCallId);
      assert.equal(typeof entry.id, 'string');
      assert.deepEqual(jsonValue(entry.toolCall), jsonValue(pending.call));
      assert.equal(observer.state(pending.chat), undefined, 'observer must not subscribe to the chat');
    },
  },
  {
    id: 'sessions/input-needed/session-only-response-clears-entry',
    area: 'Root & sessions', feature: 'inputNeeded',
    title: 'A response from a session-only subscriber resolves its aggregate entry without a chat subscription.',
    spec: '/specification/session-channel#aggregated-input-requests',
    requires: [requires.model, requires.sharedFilesystem],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const responder = await t.client({ initialSubscriptions: [] });
      await responder.subscribe(session.sessionUri);
      const pending = await heldConfirmation(t, client, session);
      const entry = await aggregate(responder, session.sessionUri, pending.chat, pending.turnId, pending.call.toolCallId);
      assert.equal(responder.state(entry.chat), undefined);
      const after = responder.lastServerSeq;
      responder.dispatch(entry.chat, confirmationAction(entry.turnId, entry.toolCall, false));
      await responder.waitForAction(session.sessionUri, (event) => event.serverSeq > after &&
        event.rejectionReason === undefined && event.action.type === 'session/inputNeededRemoved' &&
        event.action.id === entry.id, { timeoutMs: 5_000 });
      await client.waitForState<ChatState>(entry.chat, (state) => {
        const turn = state.activeTurn?.id === entry.turnId ? state.activeTurn : state.turns.find((turn) => turn.id === entry.turnId);
        const call = calls(turn).find((call) => call.toolCallId === entry.toolCall.toolCallId);
        return call !== undefined && call.status !== 'pending-confirmation' && call.status !== 'pending-result-confirmation';
      }, { timeoutMs: 5_000 });
      const snapshot = (await responder.subscribe(session.sessionUri)).state as SessionState;
      assert.ok(!snapshot.inputNeeded?.some((request) => request.id === entry.id));
    },
  },
];
