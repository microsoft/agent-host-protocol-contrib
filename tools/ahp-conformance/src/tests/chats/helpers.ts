import type { ActionEnvelope, ChatSource, ChatState, CreateChatParams, Message, PendingMessageKind, SideChatSelection, SessionState, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import type { Client, LooseStateAction, SessionHandle, TestContext } from '../../harness/types.ts';

export function userMessage(text: string): Message {
  return { text, origin: { kind: 'user' as Message['origin']['kind'] } };
}

export function dispatch(client: Client, channel: string, action: LooseStateAction): number {
  return client.dispatch(channel, action);
}

export async function accepted(client: Client, channel: string, action: LooseStateAction): Promise<ActionEnvelope> {
  const clientSeq = dispatch(client, channel, action);
  const envelope = await client.waitForAction(channel, (e) =>
    e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq);
  assert.equal(envelope.rejectionReason, undefined, envelope.rejectionReason ?? 'action was rejected');
  return envelope;
}

export function chatState(client: Client, uri: string): ChatState {
  const state = client.state<ChatState>(uri);
  assert.ok(state, `missing chat mirror for ${uri}`);
  return state;
}

export async function completeTurn(session: SessionHandle, text: string, chatUri = session.chatUri): Promise<Turn> {
  const id = session.startTurn(text, { chatUri });
  const turn = await session.waitForTurnEnd<Turn>(id, { chatUri });
  assert.equal(turn.state, 'complete', 'setup turn did not complete');
  return turn;
}

export async function createChat(
  t: TestContext,
  client: Client,
  session: SessionHandle,
  options: Partial<Pick<CreateChatParams, 'source' | 'initialMessage'>> = {},
): Promise<string> {
  const chat = `ahp-chat:/${t.uniqueId('chat')}`;
  await client.request('createChat', { channel: session.sessionUri, chat, ...options });
  await client.subscribe(chat);
  return chat;
}

export async function cancelledTurn(t: TestContext, client: Client, session: SessionHandle) {
  const text = Array.from({ length: 128 }, (_, i) => `${t.uniqueId('slow')}-${i}`).join(' ');
  const turnId = session.startTurn(prompts.reply(text, { chunks: 32, waitMs: 100 }));
  await client.waitForAction(session.chatUri, (e) =>
    (e.action.type === 'chat/delta' || e.action.type === 'chat/turnComplete' || e.action.type === 'chat/error') &&
    e.action.turnId === turnId);
  if (chatState(client, session.chatUri).activeTurn?.id !== turnId) {
    t.skip('The turn completed before cancellation could be sent.');
  }
  const clientSeq = dispatch(client, session.chatUri, { type: 'chat/turnCancelled', turnId, duration: 0 });
  const envelope = await client.waitForAction(session.chatUri, (e) =>
    e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq);
  if (envelope.rejectionReason && chatState(client, session.chatUri).turns.some((turn) =>
    turn.id === turnId && turn.state === 'complete')) {
    t.skip('The turn completed while cancellation was in flight.');
  }
  assert.equal(envelope.rejectionReason, undefined, envelope.rejectionReason ?? 'action was rejected');
  const turn = await session.waitForTurnEnd<Turn>(turnId);
  return { turnId, turn, envelope };
}

export function chatSource(kind: 'fork' | 'sideChat', chat: string, turnId: string, selection?: SideChatSelection): ChatSource {
  return { kind, chat, turnId, ...(selection ? { selection } : {}) } as ChatSource;
}

export function pendingKind(kind: 'steering' | 'queued'): PendingMessageKind {
  return kind as PendingMessageKind;
}

export async function assertChatMirrorMatchesSnapshot(
  t: TestContext,
  client: Client,
  session: SessionHandle,
  chat = session.chatUri,
): Promise<void> {
  const observer = await t.client({ initialSubscriptions: [] });
  const snapshot = (await observer.subscribe(chat)).state as ChatState;
  const summary = client.state<SessionState>(session.sessionUri)?.chats.find((entry) => entry.resource === chat);
  assert.ok(summary, `missing session catalog entry for ${chat}`);
  // AHP 1.0 has no chat title action; title updates arrive via session/chatUpdated.
  const mirror = { ...chatState(client, chat), title: summary.title };
  const normalize = (state: ChatState) => JSON.parse(JSON.stringify({
    ...state,
    // Pending-message extensions are additive; compare the fields known to 1.0.
    steeringMessage: state.steeringMessage && {
      id: state.steeringMessage.id, message: state.steeringMessage.message,
    },
    queuedMessages: state.queuedMessages?.map(({ id, message }) => ({ id, message })),
  }));
  assert.deepEqual(normalize(mirror), normalize(snapshot), `Chat mirror differs from fresh snapshot for ${chat}`);
}

export async function rejected(client: Client, channel: string, action: LooseStateAction): Promise<ActionEnvelope> {
  const clientSeq = client.dispatch(channel, action);
  const envelope = await client.waitForAction(channel, (e) =>
    e.origin?.clientId === client.clientId && e.origin.clientSeq === clientSeq);
  assert.ok(typeof envelope.rejectionReason === 'string' && envelope.rejectionReason.length > 0, 'invalid action must have a rejection reason');
  return envelope;
}

export async function snapshotState<S>(t: TestContext, uri: string): Promise<S> {
  const observer = await t.client({ initialSubscriptions: [] });
  return (await observer.subscribe(uri)).state as S;
}

export async function slowActiveTurn(t: TestContext, client: Client, session: SessionHandle, chat = session.chatUri) {
  const text = Array.from({ length: 160 }, (_, i) => `word-${i}`).join(' ');
  const id = session.startTurn(prompts.reply(text, { chunks: 40, waitMs: 100 }), { chatUri: chat });
  await client.waitForAction(chat, (e) => e.action.type === 'chat/turnStarted' && e.action.turnId === id);
  if (chatState(client, chat).activeTurn?.id !== id) t.skip('The setup turn finished before the active-turn operation could be sent.');
  return id;
}

export async function startMessage(t: TestContext, client: Client, session: SessionHandle, message: Message) {
  const turnId = t.uniqueId('turn');
  await accepted(client, session.chatUri, {
    type: 'chat/turnStarted', turnId, startedAt: new Date().toISOString(), message,
  });
  return turnId;
}

export function normalized<T>(value: T): T {
  return value === undefined ? value : JSON.parse(JSON.stringify(value)) as T;
}
