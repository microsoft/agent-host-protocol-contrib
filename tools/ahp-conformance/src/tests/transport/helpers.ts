import { PROTOCOL_VERSION } from '@microsoft/agent-host-protocol';
import type {
  InitializeResult,
  ReconnectResult,
  Snapshot,
} from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import type { Client, Connection, TestContext } from '../../harness/types.ts';

export const ROOT = 'ahp-root://';

export function initialize(
  t: TestContext,
  connection: Connection,
  overrides: Record<string, unknown> = {},
): Promise<InitializeResult> {
  return connection.request<InitializeResult>('initialize', {
    channel: ROOT,
    clientId: t.uniqueId('transport-client'),
    protocolVersions: [PROTOCOL_VERSION],
    initialSubscriptions: [ROOT],
    ...overrides,
  });
}

export function assertSnapshot(snapshot: Snapshot | undefined, resource: string): asserts snapshot is Snapshot {
  assert.ok(snapshot, `missing snapshot for ${resource}`);
  assert.equal(snapshot.resource, resource);
  assert.equal(typeof snapshot.fromSeq, 'number');
  assert.ok(Number.isFinite(snapshot.fromSeq));
  assert.ok(snapshot.state !== null && typeof snapshot.state === 'object' && !Array.isArray(snapshot.state));
}

export async function reconnect(
  t: TestContext,
  clientId: string,
  lastSeenServerSeq: number,
  subscriptions: string[],
): Promise<{ connection: Connection; result: ReconnectResult }> {
  const connection = await t.connect({ label: 'reconnected' });
  const result = await connection.request<ReconnectResult>('reconnect', {
    channel: ROOT,
    clientId,
    lastSeenServerSeq,
    subscriptions,
  });
  assert.ok(result.type === 'replay' || result.type === 'snapshot', 'invalid reconnect result discriminant');
  if (result.type === 'replay') {
    assert.ok(Array.isArray(result.actions));
    assert.ok(Array.isArray(result.missing));
  } else {
    assert.ok(Array.isArray(result.snapshots));
  }
  return { connection, result };
}

export function requireReplay(t: TestContext, result: ReconnectResult) {
  if (result.type !== 'replay') t.skip('The host selected snapshot recovery instead of retaining a replayable gap.');
  assert.ok(Array.isArray(result.actions));
  return result;
}

export async function missedChatActions(t: TestContext) {
  const writer = await t.client({ label: 'writer' });
  const session = await t.session(writer);
  const observer = await t.client({ label: 'observer', initialSubscriptions: [session.chatUri] });
  const lastSeenServerSeq = observer.lastServerSeq;
  await observer.close();
  const turnId = session.startTurn(prompts.reply(t.uniqueId('missed-reply')));
  await session.waitForTurnEnd(turnId);
  const expected = writer.actions(session.chatUri).filter((action) => action.serverSeq > lastSeenServerSeq);
  assert.ok(expected.length >= 2, 'the writer did not observe the turn actions');
  const finalSeq = expected.at(-1)!.serverSeq;
  const resumed = await reconnect(t, observer.clientId, lastSeenServerSeq, [session.chatUri]);
  if (resumed.result.type === 'replay' && resumed.result.missing.includes(session.chatUri)) {
    t.skip('The host could not resume access to this chat on the new connection.');
  }
  return { ...resumed, session, turnId, lastSeenServerSeq, finalSeq, expected };
}

export async function expectNoResponses(connection: Connection, send: () => void): Promise<void> {
  const start = connection.received.length;
  send();
  // This provides a bounded observation window without assuming a response timeout error shape.
  await connection.expectNoNotification('action', () => false, 350);
  const responses = connection.received.slice(start).filter((message) =>
    Object.hasOwn(message, 'result') || Object.hasOwn(message, 'error'));
  assert.deepEqual(responses, [], 'a notification received a JSON-RPC response');
}

export async function waitForClose(t: TestContext, connection: Connection): Promise<void> {
  t.signal.throwIfAborted();
  let onAbort: () => void = () => {};
  const aborted = new Promise<never>((_resolve, reject) => {
    onAbort = () => reject(t.signal.reason);
    t.signal.addEventListener('abort', onAbort, { once: true });
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timedOut = new Promise<never>((_resolve, reject) => {
    timer = setTimeout(() => reject(new Error(
      'The host did not close the connection within 5000 ms after UnsupportedProtocolVersion.',
    )), 5_000);
  });
  try {
    await Promise.race([connection.closed, aborted, timedOut]);
  } finally {
    clearTimeout(timer);
    t.signal.removeEventListener('abort', onAbort);
  }
}

export async function renameForSetup(t: TestContext, client: Client, uri: string, title: string): Promise<void> {
  const clientSeq = client.dispatch(uri, { type: 'session/titleChanged', title });
  const echo = await client.waitForAction(uri, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
  if (echo.rejectionReason !== undefined) {
    t.skip('The setup rename was rejected; session-title acceptance is covered by sessions/session-actions/title-changed.');
  }
}
