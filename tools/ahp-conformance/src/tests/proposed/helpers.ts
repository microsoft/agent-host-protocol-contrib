import { assert } from '../../harness/assert.ts';
import { proposedClient } from '../../harness/proposed.ts';
import type { Client, Connection, JsonRpcMessage, TestContext } from '../../harness/types.ts';

export const headSpec = (path: string, section: string) =>
  `https://github.com/microsoft/agent-host-protocol/blob/63c52b8a/${path}#${section}`;

export function record(value: unknown): Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
  return value as Record<string, unknown>;
}

export interface WireSnapshot {
  resource: string;
  state: Record<string, unknown>;
  fromSeq: number;
}

export interface WireEnvelope {
  channel: string;
  action: Record<string, unknown>;
  serverSeq: number;
  origin?: { clientId: string; clientSeq: number };
  rejectionReason?: string;
}

export async function snapshot(client: Connection, channel: string): Promise<WireSnapshot> {
  const result = record(await client.request('subscribe', { channel }));
  const value = record(result.snapshot);
  assert.equal(value.resource, channel);
  assert.equal(typeof value.fromSeq, 'number');
  return { resource: channel, state: record(value.state), fromSeq: value.fromSeq as number };
}

export function after(client: Connection, cursor: number, predicate: (message: JsonRpcMessage) => boolean) {
  return (message: JsonRpcMessage) => client.received.indexOf(message) >= cursor && predicate(message);
}

const sequences = new WeakMap<Connection, number>();

export function dispatch(client: Client, channel: string, action: Record<string, unknown>): number {
  const seen = client.received.flatMap((message) => {
    if (message.method !== 'action') return [];
    const envelope = message.params as WireEnvelope;
    return envelope.origin?.clientId === client.clientId ? [envelope.origin.clientSeq] : [];
  });
  const clientSeq = Math.max(sequences.get(client) ?? 0, ...seen) + 1;
  sequences.set(client, clientSeq);
  client.notify('dispatchAction', { channel, clientSeq, action });
  return clientSeq;
}

export async function accepted(client: Client, channel: string, action: Record<string, unknown>, allowRejected = false): Promise<WireEnvelope> {
  const cursor = client.received.length;
  const clientSeq = dispatch(client, channel, action);
  const message = await client.waitForMessage(after(client, cursor, (message) => {
    if (message.method !== 'action') return false;
    const envelope = message.params as WireEnvelope;
    return envelope.channel === channel && envelope.origin?.clientId === client.clientId
      && envelope.origin.clientSeq === clientSeq;
  }));
  const envelope = message.params as WireEnvelope;
  if (!allowRejected) assert.equal(envelope.rejectionReason, undefined, envelope.rejectionReason ?? 'action rejected');
  return envelope;
}

export const userMessage = (text: string) => ({ text, origin: { kind: 'user' } });

export async function sessionSetup(t: TestContext) {
  const owner = await proposedClient(t);
  const session = await t.session(owner);
  await session.ready();
  return { owner, session };
}

export async function createChat(t: TestContext, client: Client, session: string, extra: Record<string, unknown> = {}) {
  const channel = `ahp-chat:/${t.uniqueId('proposed-chat')}`;
  await client.request('createChat', { channel: session, chat: channel, ...extra });
  t.defer(async () => {
    try { await client.request('disposeChat', { channel }, { timeoutMs: 2000 }); }
    catch { /* Owning session cleanup also disposes its chats. */ }
  });
  return channel;
}

export function startTurn(t: TestContext, client: Client, chat: string, text: string): string {
  const turnId = t.uniqueId('proposed-turn');
  dispatch(client, chat, {
    type: 'chat/turnStarted', turnId, startedAt: new Date().toISOString(),
    message: { ...userMessage(text), ...(t.profile.model && { model: { id: t.profile.model.id } }) },
  });
  return turnId;
}

export async function turnEnded(client: Connection, chat: string, turnId: string) {
  await client.waitForMessage((message) => {
    if (message.method !== 'action') return false;
    const envelope = message.params as WireEnvelope;
    return envelope.channel === chat && envelope.action.turnId === turnId
      && ['chat/turnComplete', 'chat/error', 'chat/turnCancelled'].includes(String(envelope.action.type));
  });
  const state = (await snapshot(client, chat)).state;
  const turns = state.turns as Record<string, unknown>[];
  const turn = turns.find((entry) => entry.id === turnId);
  assert.ok(turn, `missing finished turn ${turnId}`);
  assert.equal(turn.state, 'complete', 'setup turn did not complete');
  return turn;
}
