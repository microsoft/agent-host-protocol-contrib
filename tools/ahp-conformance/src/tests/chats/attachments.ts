import type { ChatState, MessageChatAttachment, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { chatState, completeTurn, normalized, slowActiveTurn, snapshotState, startMessage, userMessage } from './helpers.ts';

function attachment(resource: string, endTurn?: string): MessageChatAttachment {
  return { type: 'chat' as MessageChatAttachment['type'], resource, label: 'Referenced chat', ...(endTurn ? { endTurn } : {}) };
}

export const tests: TestDefinition[] = [
  ...(['explicit-end-turn', 'omitted-end-turn'] as const).map((kind): TestDefinition => ({
    id: `chats/attachments/${kind}-recalls-pinned-transcript`, area: 'Chats & turns', feature: 'chatAttachment',
    title: `A chat attachment with ${kind.replaceAll('-', ' ')} supplies a fixed transcript as model context.`,
    spec: '/specification/chat-channel#pulling-a-chat-into-another-chat', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const target = await t.session(client);
      await source.ready();
      await target.ready();
      const phrase = t.uniqueId('attached-memory');
      const bound = await completeTurn(source, prompts.remember(phrase));
      if (kind === 'explicit-end-turn') await completeTurn(source, prompts.remember(t.uniqueId('beyond-bound')));
      const message = { ...userMessage(prompts.recall()), attachments: [attachment(source.chatUri, kind === 'explicit-end-turn' ? bound.id : undefined)] };
      const id = await startMessage(t, client, target, message);
      const turn = await target.waitForTurnEnd<Turn>(id);
      assert.equal(turn.state, 'complete');
      assert.ok(target.responseText(id).trim().includes(phrase), 'attached transcript did not supply the bounded remembered phrase');
      await completeTurn(source, prompts.remember(t.uniqueId('after-acceptance')));
      const recalled = await completeTurn(target, prompts.recall());
      assert.ok(target.responseText(recalled.id).trim().includes(phrase), 'later source turns changed the already accepted attachment context');
    },
  })),
  ...(['unknown-chat', 'unknown-end-turn', 'active-end-turn', 'non-retained-end-turn'] as const).map((kind): TestDefinition => ({
    id: `chats/attachments/${kind}-rejected`, area: 'Chats & turns', feature: 'chatAttachment',
    title: `A chat attachment with ${kind.replaceAll('-', ' ')} is rejected before starting a turn.`,
    spec: '/specification/chat-channel#pulling-a-chat-into-another-chat',
    requires: kind === 'unknown-chat' || kind === 'unknown-end-turn' ? [] : [requires.model],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const target = await t.session(client);
      await source.ready();
      await target.ready();
      let resource = source.chatUri;
      let endTurn: string | undefined;
      if (kind === 'unknown-chat') resource = `ahp-chat:/${t.uniqueId('missing')}`;
      else if (kind === 'unknown-end-turn') endTurn = t.uniqueId('missing-turn');
      else if (kind === 'active-end-turn') endTurn = await slowActiveTurn(t, client, source);
      else {
        const turn = await completeTurn(source, prompts.reply(t.uniqueId('pruned')));
        const seq = client.dispatch(source.chatUri, { type: 'chat/truncated' });
        const echo = await client.waitForAction(source.chatUri, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === seq);
        if (echo.rejectionReason || (await snapshotState<ChatState>(t, source.chatUri)).turns.some((entry) => entry.id === turn.id)) {
          t.skip('The host did not remove the setup turn; truncation acceptance is covered separately.');
        }
        endTurn = turn.id;
      }
      const before = await snapshotState<ChatState>(t, target.chatUri);
      const turnId = t.uniqueId('invalid-attachment');
      const seq = client.dispatch(target.chatUri, {
        type: 'chat/turnStarted', turnId, startedAt: new Date().toISOString(),
        message: { ...userMessage(prompts.reply('not-started')), attachments: [attachment(resource, endTurn)] },
      });
      const echo = await client.waitForAction(target.chatUri, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === seq);
      if (kind === 'active-end-turn' && !echo.rejectionReason && chatState(client, source.chatUri).activeTurn?.id !== endTurn) {
        t.skip('The referenced turn completed while the attachment was in flight.');
      }
      assert.ok(echo.rejectionReason, 'invalid chat attachment must be rejected');
      assert.deepEqual(normalized(await snapshotState<ChatState>(t, target.chatUri)), normalized(before));
    },
  })),
  {
    id: 'chats/attachments/empty-transcript-accepted', area: 'Chats & turns', feature: 'chatAttachment',
    title: 'A known chat without completed retained turns can be attached as an empty transcript.',
    spec: '/specification/chat-channel#pulling-a-chat-into-another-chat', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      const sourceSession = await t.session(client);
      await session.ready();
      await sourceSession.ready();
      const source = sourceSession.chatUri;
      if ((await snapshotState<ChatState>(t, source)).turns.length > 0) {
        const seq = client.dispatch(source, { type: 'chat/truncated' });
        const echo = await client.waitForAction(source, (e) => e.origin?.clientId === client.clientId && e.origin.clientSeq === seq);
        if (echo.rejectionReason || (await snapshotState<ChatState>(t, source)).turns.length > 0) {
          t.skip('The lazy-initialization warm-up could not be removed to create an empty-transcript precondition.');
        }
      }
      const id = await startMessage(t, client, session, {
        ...userMessage(prompts.reply(t.uniqueId('empty-attachment'))), attachments: [attachment(source)],
      });
      assert.equal((await session.waitForTurnEnd<Turn>(id)).state, 'complete');
    },
  },
];
