import type { ChatState, SessionState, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { additionalChat, ERROR, heldConfirmation, INPUT_NEEDED } from './coverage-helpers.ts';
import { listedSession, sessionState } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'sessions/summary/nondefault-input-promoted',
    area: 'Root & sessions', feature: 'sessionSummary',
    title: 'SHOULD: A non-default chat awaiting input promotes session status while the default chat is idle.',
    spec: '/specification/session-channel#chat-aggregation',
    requires: [requires.multipleChats, requires.model, requires.sharedFilesystem],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const initial = sessionState(client, session.sessionUri);
      if (initial.defaultChat !== session.chatUri) t.skip('The session does not designate the initial chat as default.');
      const chat = await additionalChat(t, client, session);
      await heldConfirmation(t, client, session, chat);
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.equal(client.state<ChatState>(session.chatUri)?.activeTurn, undefined);
      assert.equal(snapshot.status & INPUT_NEEDED, INPUT_NEEDED);
      const summary = await listedSession(t, client, session.sessionUri);
      assert.equal(summary.status & INPUT_NEEDED, INPUT_NEEDED);
    },
  },
  {
    id: 'sessions/summary/nondefault-error-promoted',
    area: 'Root & sessions', feature: 'sessionSummary',
    title: 'SHOULD: An errored non-default chat promotes session Error while the default chat is idle.',
    spec: '/specification/session-channel#chat-aggregation',
    requires: [requires.multipleChats, requires.model, requires.deterministicModel],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      if (sessionState(client, session.sessionUri).defaultChat !== session.chatUri) {
        t.skip('The session does not designate the initial chat as default.');
      }
      const chat = await additionalChat(t, client, session);
      const turnId = session.startTurn(prompts.upstreamError(503, t.uniqueId('error')), { chatUri: chat });
      const turn = await session.waitForTurnEnd<Turn>(turnId, { chatUri: chat });
      if (turn.state !== 'error') t.skip('The deterministic upstream-error setup did not produce an errored turn.');
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = (await observer.subscribe(session.sessionUri)).state as SessionState;
      assert.equal(client.state<ChatState>(session.chatUri)?.activeTurn, undefined);
      assert.notEqual(snapshot.status & ERROR, 0);
      assert.notEqual((await listedSession(t, client, session.sessionUri)).status & ERROR, 0);
    },
  },
  {
    id: 'sessions/summary/modified-at-is-chat-maximum',
    area: 'Root & sessions', feature: 'sessionSummary',
    title: 'SHOULD: Session modification time is the maximum chat modification time, including non-default chats.',
    spec: '/specification/session-channel#chat-aggregation',
    requires: [requires.multipleChats, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const chat = await additionalChat(t, client, session);
      const turnId = session.startTurn(prompts.reply(t.uniqueId('reply')), { chatUri: chat });
      const turn = await session.waitForTurnEnd<Turn>(turnId, { chatUri: chat });
      if (turn.state !== 'complete') t.skip('The prerequisite reply turn did not complete successfully.');
      const observer = await t.client({ initialSubscriptions: [] });
      const snapshot = (await observer.subscribe(session.sessionUri)).state as SessionState;
      const times = snapshot.chats.map((entry) => Date.parse(entry.modifiedAt));
      assert.ok(times.length > 0 && times.every(Number.isFinite));
      const summary = await listedSession(t, client, session.sessionUri);
      assert.equal(Date.parse(summary.modifiedAt), Math.max(...times));
    },
  },
];
