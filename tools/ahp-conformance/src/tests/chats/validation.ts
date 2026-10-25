import type { ActionEnvelope, ChatInputResponseKind, ChatState, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { LooseStateAction, Requirement, TestDefinition } from '../../harness/types.ts';
import { chatSource, completeTurn, normalized, rejected, slowActiveTurn, snapshotState } from './helpers.ts';
import { RpcError } from '../../harness/types.ts';

export const tests: TestDefinition[] = [
  {
    id: 'chats/actions/unknown-chat-no-echo', area: 'Chats & turns', feature: 'dispatchAction',
    title: 'An action targeting an unknown chat is silently ignored without an echo.',
    spec: '/specification/chat-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const chat = `ahp-chat:/${t.uniqueId('missing')}`;
      const clientSeq = client.dispatch(chat, { type: 'chat/isReadChanged', isRead: true });
      await client.request('ping', { channel: 'ahp-root://' });
      await client.expectNoNotification<ActionEnvelope>('action', (e) => e.channel === chat ||
        (e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq), 400);
    },
  },
  ...(['inputAnswerChanged', 'inputCompleted'] as const).map((action): TestDefinition => ({
    id: `chats/input/${action === 'inputAnswerChanged' ? 'unknown-answer-request-rejected' : 'unknown-completion-request-rejected'}`,
    area: 'Chats & turns', feature: action,
    title: `SHOULD: An ${action} action for an unknown input request is rejected without state changes.`,
    spec: '/specification/chat-channel#server-validation-of-client-actions',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const before = await snapshotState<ChatState>(t, session.chatUri);
      const requestId = t.uniqueId('unknown-request');
      const input: LooseStateAction = action === 'inputAnswerChanged'
        ? { type: 'chat/inputAnswerChanged', requestId, questionId: t.uniqueId('question') }
        : { type: 'chat/inputCompleted', requestId, response: 'cancel' as ChatInputResponseKind };
      await rejected(client, session.chatUri, input);
      assert.deepEqual(normalized(await snapshotState<ChatState>(t, session.chatUri)), normalized(before));
    },
  })),
  ...(['fork', 'sideChat'] as const).map((kind): TestDefinition => ({
    id: `chats/create-chat/unsupported-${kind === 'sideChat' ? 'side-chat' : kind}-rejected`,
    area: 'Chats & turns', feature: 'createChat',
    title: `Creating a ${kind} source is rejected when the corresponding capability is not advertised.`,
    spec: '/specification/chat-channel#creation',
    requires: [requires.model, {
      label: `multipleChats.${kind} absent`, kind: 'capability',
      check: (p) => p.agent.capabilities?.multipleChats?.[kind] !== true,
    } satisfies Requirement],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turn = await completeTurn(session, prompts.reply(t.uniqueId('source')));
      const chat = `ahp-chat:/${t.uniqueId('rejected')}`;
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat, source: chatSource(kind, session.chatUri, turn.id),
      }), RpcError);
      const owner = await snapshotState<SessionState>(t, session.sessionUri);
      assert.ok(!owner.chats.some((entry) => entry.resource === chat));
    },
  })),
  ...(['fork', 'sideChat'] as const).map((kind): TestDefinition => ({
    id: `chats/create-chat/${kind === 'sideChat' ? 'side-chat' : kind}-self-source-rejected`,
    area: 'Chats & turns', feature: 'createChat',
    title: `A ${kind} creation cannot name the newly requested chat as its own source.`,
    spec: '/specification/chat-channel#creation', requires: [kind === 'fork' ? requires.fork : requires.sideChat],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const chat = `ahp-chat:/${t.uniqueId('self-source')}`;
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat, source: chatSource(kind, chat, t.uniqueId('turn')),
      }), RpcError);
      assert.ok(!(await snapshotState<SessionState>(t, session.sessionUri)).chats.some((entry) => entry.resource === chat));
    },
  })),
  {
    id: 'chats/fork/active-turn-rejected', area: 'Chats & turns', feature: 'fork',
    title: 'A fork cannot target the source chat’s active turn.',
    spec: '/specification/chat-channel#creation', requires: [requires.fork, requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const turnId = await slowActiveTurn(t, client, session);
      const chat = `ahp-chat:/${t.uniqueId('fork')}`;
      try {
        await client.request('createChat', { channel: session.sessionUri, chat, source: chatSource('fork', session.chatUri, turnId) });
      } catch (error) {
        assert.ok(error instanceof RpcError);
        return;
      }
      const current = await snapshotState<ChatState>(t, session.chatUri);
      if (current.activeTurn?.id !== turnId) t.skip('The source turn completed while createChat was in flight.');
      assert.fail('fork creation accepted a still-active source turn');
    },
  },
  {
    id: 'chats/side-chat/unknown-chat-rejected', area: 'Chats & turns', feature: 'sideChat',
    title: 'Side-chat creation rejects an unknown source chat.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('side')}`,
        source: chatSource('sideChat', `ahp-chat:/${t.uniqueId('missing')}`, t.uniqueId('turn')),
      }), RpcError);
    },
  },
  {
    id: 'chats/side-chat/cross-session-source-rejected', area: 'Chats & turns', feature: 'sideChat',
    title: 'Side-chat creation rejects a source chat belonging to another session.',
    spec: '/specification/chat-channel#creation', requires: [requires.sideChat, requires.model],
    async run(t) {
      const client = await t.client();
      const source = await t.session(client);
      const target = await t.session(client);
      await source.ready();
      await target.ready();
      const turn = await completeTurn(source, prompts.reply(t.uniqueId('source')));
      await assert.rejects(client.request('createChat', {
        channel: target.sessionUri, chat: `ahp-chat:/${t.uniqueId('side')}`,
        source: chatSource('sideChat', source.chatUri, turn.id),
      }), RpcError);
    },
  },
];
