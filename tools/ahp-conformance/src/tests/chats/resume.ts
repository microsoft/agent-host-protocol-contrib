import type { ActionEnvelope, ChatState, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';
import { accepted, completeTurn, normalized, rejected, slowActiveTurn, snapshotState } from './helpers.ts';

async function errorTurn(t: TestContext, session: SessionHandle): Promise<Turn> {
  const id = session.startTurn(prompts.upstreamError(400, t.uniqueId('resume-error')));
  const turn = await session.waitForTurnEnd<Turn>(id);
  assert.equal(turn.state, 'error', 'the deterministic upstream error must produce an errored turn');
  assert.equal(turn.responseParts.at(-1)?.kind, 'error');
  return turn;
}

function resumable(t: TestContext, turn: Turn) {
  const part = turn.responseParts.at(-1);
  if (part?.kind !== 'error' || part.resumable !== true) t.skip('The host did not offer a resumable error part for the controlled upstream failure.');
}

async function noLaterEffects(client: Client, chat: string, turnId: string, serverSeq: number) {
  await client.expectNoNotification<ActionEnvelope>('action', (e) => e.channel === chat && e.serverSeq > serverSeq && !e.rejectionReason &&
    'turnId' in e.action && e.action.turnId === turnId, 400);
}

export const tests: TestDefinition[] = [
  {
    id: 'chats/turn-resume/preserves-turn-and-prior-errors', area: 'Chats & turns', feature: 'turnResume',
    title: 'Resuming an offered resumable error retains the same turn and appends any subsequent error in stream order.',
    spec: '/specification/chat-channel#error-recovery', requires: [requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const prior = await errorTurn(t, session);
      resumable(t, prior);
      const before = await snapshotState<ChatState>(t, session.chatUri);
      const beforeResumeSeq = client.lastServerSeq;
      const echo = await accepted(client, session.chatUri, { type: 'chat/turnResume', turnId: prior.id });
      const state = await client.waitForState<ChatState>(session.chatUri, (chat) => {
        const turn = chat.turns.find((entry) => entry.id === prior.id);
        return chat.activeTurn?.id !== prior.id && !!turn && (turn.state === 'complete' || turn.responseParts.length > prior.responseParts.length);
      });
      const resumed = state.turns.find((entry) => entry.id === prior.id)!;
      assert.deepEqual(normalized(resumed.message), normalized(prior.message), 'resume must not synthesize a new user message');
      assert.deepEqual(normalized(resumed.responseParts.slice(0, prior.responseParts.length)), normalized(prior.responseParts), 'resume lost the previous response/error prefix');
      assert.deepEqual(state.turns.map((turn) => turn.id), before.turns.map((turn) => turn.id));
      const effects = client.actions(session.chatUri).filter((e) => e.serverSeq > beforeResumeSeq && !e.rejectionReason && e.action.type !== 'chat/turnResume' && 'turnId' in e.action && e.action.turnId === prior.id);
      for (const effect of effects) assert.ok(effect.serverSeq > echo.serverSeq, 'resume must be sequenced before continuation effects');
      const subsequent = client.actions(session.chatUri).filter((e) => e.serverSeq > echo.serverSeq && !e.rejectionReason);
      assert.ok(!subsequent.some((e) => e.action.type === 'chat/turnStarted'), 'resume must not create a synthetic turn');
      if (!subsequent.some((e) => e.action.type === 'chat/usage' && e.action.turnId === prior.id)) {
        assert.deepEqual(normalized({ usage: resumed.usage }), normalized({ usage: prior.usage }));
      }
      if (resumed.state === 'error') {
        assert.ok(resumed.responseParts.length > prior.responseParts.length);
        assert.equal(resumed.responseParts.at(-1)?.kind, 'error');
      } else assert.equal(resumed.state, 'complete');
      const fresh = await snapshotState<ChatState>(t, session.chatUri);
      assert.deepEqual(normalized(fresh.turns.find((turn) => turn.id === prior.id)), normalized(resumed));
    },
  },
  {
    id: 'chats/turn-resume/completed-turn-has-no-effects', area: 'Chats & turns', feature: 'turnResume',
    title: 'Rejecting resume of a completed turn produces no later turn actions or state changes.',
    spec: '/specification/chat-channel#error-recovery', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await completeTurn(session, prompts.reply(t.uniqueId('complete')));
      const before = await snapshotState<ChatState>(t, session.chatUri);
      const beforeResumeSeq = client.lastServerSeq;
      await rejected(client, session.chatUri, { type: 'chat/turnResume', turnId: turn.id });
      await noLaterEffects(client, session.chatUri, turn.id, beforeResumeSeq);
      assert.deepEqual(normalized(await snapshotState<ChatState>(t, session.chatUri)), normalized(before));
    },
  },
  {
    id: 'chats/turn-resume/non-resumable-error-has-no-effects', area: 'Chats & turns', feature: 'turnResume',
    title: 'A latest error that does not offer resume is rejected without changing the turn.',
    spec: '/specification/chat-channel#server-validation-of-client-actions', requires: [requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await errorTurn(t, session);
      const part = turn.responseParts.at(-1);
      if (part?.kind === 'error' && part.resumable === true) t.skip('The host offered resume; this case requires a non-resumable final error.');
      const before = await snapshotState<ChatState>(t, session.chatUri);
      const beforeResumeSeq = client.lastServerSeq;
      await rejected(client, session.chatUri, { type: 'chat/turnResume', turnId: turn.id });
      await noLaterEffects(client, session.chatUri, turn.id, beforeResumeSeq);
      assert.deepEqual(normalized(await snapshotState<ChatState>(t, session.chatUri)), normalized(before));
    },
  },
  ...(['stale-error', 'active-turn'] as const).map((kind): TestDefinition => ({
    id: `chats/turn-resume/${kind}-rejected-without-effects`, area: 'Chats & turns', feature: 'turnResume',
    title: `A resume request with ${kind === 'stale-error' ? 'a stale resumable error' : 'another active turn'} is rejected without restarting the error turn.`,
    spec: '/specification/chat-channel#server-validation-of-client-actions', requires: [requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await errorTurn(t, session);
      resumable(t, turn);
      const activeId = kind === 'active-turn' ? await slowActiveTurn(t, client, session) : undefined;
      if (kind === 'stale-error') await completeTurn(session, prompts.reply(t.uniqueId('newer-turn')));
      const before = await snapshotState<ChatState>(t, session.chatUri);
      const beforeResumeSeq = client.lastServerSeq;
      await rejected(client, session.chatUri, { type: 'chat/turnResume', turnId: turn.id });
      await noLaterEffects(client, session.chatUri, turn.id, beforeResumeSeq);
      const after = await snapshotState<ChatState>(t, session.chatUri);
      assert.deepEqual(normalized(after.turns.find((entry) => entry.id === turn.id)), normalized(before.turns.find((entry) => entry.id === turn.id)));
      if (!activeId) assert.deepEqual(normalized(after), normalized(before));
    },
  })),
];
