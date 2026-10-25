import { assert } from '../../harness/assert.ts';
import { proposedClient } from '../../harness/proposed.ts';
import type { Client, Connection, JsonRpcMessage, TestContext } from '../../harness/types.ts';
import { accepted, after, record, sessionSetup } from './helpers.ts';

export interface Limits { windowBytes: number; maximumFrameBytes: number }
export interface FlowControl { clientReceive: Limits; hostReceive: Limits }
export interface WindowResult { flowControl?: FlowControl; snapshot?: unknown }

const exempt = new Set(['channel/credit', 'unsubscribe', 'tcp/reset']);
const isNotification = (message: JsonRpcMessage) => typeof message.method === 'string' && message.id === undefined;
export const channelOf = (message: JsonRpcMessage) => (message.params as { channel?: string } | undefined)?.channel;

export function wireBytes(client: Connection, message: JsonRpcMessage): number {
  const frame = client.receivedFrames.find((entry) => entry.message === message);
  assert.ok(frame, 'received JSON message has no exact transport-frame record');
  assert.equal(frame.utf16Bytes, frame.text.length * 2, 'raw frame UTF-16 accounting mismatch');
  return frame.utf16Bytes;
}

export function assertLimits(limits: Limits, offered?: Limits): void {
  assert.ok(Number.isSafeInteger(limits.windowBytes) && limits.windowBytes >= 1, 'invalid windowBytes');
  assert.ok(Number.isSafeInteger(limits.maximumFrameBytes) && limits.maximumFrameBytes >= 1024, 'invalid maximumFrameBytes');
  if (offered) {
    assert.ok(limits.windowBytes <= offered.windowBytes, 'host raised offered window');
    assert.ok(limits.maximumFrameBytes <= offered.maximumFrameBytes, 'host raised offered frame limit');
  }
}

export async function offerWindow(client: Connection, channel: string, receive: Limits) {
  const cursor = client.received.length;
  const result = await client.request<WindowResult>('subscribe', { channel, flowControl: { receive } });
  if (!result.flowControl) return { result, cursor };
  assertLimits(result.flowControl.clientReceive, receive);
  assertLimits(result.flowControl.hostReceive);
  assert.equal(result.snapshot, undefined, 'windowed subscribe embedded a snapshot in response');
  const responseIndex = client.received.findIndex((message, index) => index >= cursor && message.result === result);
  assert.ok(responseIndex >= cursor, 'subscribe response absent from received messages');
  const early = client.received.slice(cursor, responseIndex).filter((message) =>
    isNotification(message) && channelOf(message) === channel && !exempt.has(String(message.method)));
  assert.deepEqual(early, [], 'subscription data preceded subscribe response');
  return { result, cursor };
}

export class WindowPeer {
  readonly connection: Connection;
  readonly channel: string;
  readonly flow: FlowControl;
  readonly logical: JsonRpcMessage[] = [];
  readonly dataFrames: JsonRpcMessage[] = [];
  receivedBytes = 0;
  releasedBytes = 0;
  sentBytes = 0;
  hostConsumedBytes = 0;
  private cursor: number;
  private fragment = '';
  private previousFragment = '';

  constructor(connection: Connection, channel: string, flow: FlowControl, cursor: number) {
    this.connection = connection;
    this.channel = channel;
    this.flow = flow;
    this.cursor = cursor;
  }

  get outstanding() { return this.receivedBytes - this.releasedBytes; }
  get hostOutstanding() { return this.sentBytes - this.hostConsumedBytes; }

  poll(): void {
    const received = this.connection.received;
    while (this.cursor < received.length) {
      const message = received[this.cursor++]!;
      if (!isNotification(message) || channelOf(message) !== this.channel) continue;
      if (message.method === 'channel/credit') {
        const total = (message.params as { consumedBytes: number }).consumedBytes;
        assert.ok(Number.isSafeInteger(total) && total >= 0 && total <= this.sentBytes, 'invalid host credit total');
        this.hostConsumedBytes = Math.max(this.hostConsumedBytes, total);
        continue;
      }
      if (exempt.has(String(message.method))) continue;
      const bytes = wireBytes(this.connection, message);
      assert.ok(bytes <= this.flow.clientReceive.maximumFrameBytes, 'data frame exceeds negotiated maximumFrameBytes');
      assert.ok(this.outstanding < this.flow.clientReceive.windowBytes, 'host sent data at a full window');
      this.receivedBytes += bytes;
      this.dataFrames.push(message);
      if (message.method === 'channel/frame') {
        const params = record(message.params);
        assert.equal(typeof params.data, 'string');
        const data = params.data as string;
        assert.ok(data.length > 0, 'empty fragment');
        const high = this.previousFragment.charCodeAt(this.previousFragment.length - 1);
        const low = data.charCodeAt(0);
        assert.ok(!(high >= 0xd800 && high <= 0xdbff && low >= 0xdc00 && low <= 0xdfff), 'fragment split a surrogate pair');
        this.fragment += data;
        this.previousFragment = data;
        if (params.final === true) {
          const joined = record(JSON.parse(this.fragment)) as JsonRpcMessage;
          assert.equal(joined.jsonrpc, '2.0');
          assert.ok(isNotification(joined), 'fragmented request/response');
          assert.ok(!exempt.has(String(joined.method)), 'fragmented control notification');
          assert.notEqual(joined.method, 'channel/frame', 'nested fragments');
          assert.equal(channelOf(joined), this.channel, 'joined channel mismatch');
          this.logical.push(joined);
          this.fragment = '';
          this.previousFragment = '';
        }
      } else {
        assert.equal(this.fragment, '', 'same-subscription data interleaved with incomplete fragments');
        this.logical.push(message);
      }
    }
  }

  credit(total = this.receivedBytes): void {
    assert.ok(total >= 0 && total <= this.receivedBytes, 'test attempted over-credit');
    this.connection.notify('channel/credit', { channel: this.channel, consumedBytes: total });
    this.releasedBytes = Math.max(this.releasedBytes, total);
  }

  async changed(timeoutMs?: number): Promise<void> {
    const cursor = this.connection.received.length;
    await this.connection.waitForMessage(after(this.connection, cursor, (message) =>
      isNotification(message) && channelOf(message) === this.channel), { timeoutMs });
    this.poll();
  }

  async until(predicate: () => boolean, release = true): Promise<void> {
    this.poll();
    while (!predicate()) {
      if (release && this.receivedBytes > this.releasedBytes) this.credit();
      await this.changed();
    }
  }

  async initialSnapshot() {
    await this.until(() => this.logical.length > 0);
    const message = this.logical[0]!;
    assert.equal(message.method, 'channel/snapshot', 'first window data was not snapshot');
    const snapshot = record(record(message.params).snapshot);
    assert.equal(snapshot.resource, this.channel);
    assert.equal(typeof snapshot.fromSeq, 'number');
    record(snapshot.state);
    return snapshot;
  }

  async paused(): Promise<void> {
    await this.until(() => this.outstanding >= this.flow.clientReceive.windowBytes, false);
    assert.ok(this.outstanding < this.flow.clientReceive.windowBytes + this.flow.clientReceive.maximumFrameBytes);
  }

  async quiet(): Promise<void> {
    this.poll();
    const cursor = this.connection.received.length;
    await Promise.all(['channel/frame', 'channel/snapshot', 'action'].map((method) =>
      this.connection.expectNoNotification(method, (params) =>
        (params as { channel?: string }).channel === this.channel
        && this.connection.received.findLastIndex((message) => message.method === method && message.params === params) >= cursor, 350)));
    this.poll();
  }

  async sendText(text: string, release = true): Promise<void> {
    const bytes = text.length * 2;
    assert.ok(bytes <= this.flow.hostReceive.maximumFrameBytes, 'test sent oversized frame');
    this.poll();
    while (this.hostOutstanding >= this.flow.hostReceive.windowBytes) {
      if (release && this.outstanding > 0) this.credit();
      await this.changed();
    }
    this.connection.sendRaw(text);
    this.sentBytes += bytes;
  }

  async sendFragment(data: string, final = false, release = true): Promise<void> {
    await this.sendText(JSON.stringify({ jsonrpc: '2.0', method: 'channel/frame', params: {
      channel: this.channel, data, ...(final && { final: true }),
    } }), release);
  }

  async sendJoined(text: string, release = true): Promise<void> {
    const chunks = fragmentText(this.channel, text, this.flow.hostReceive.maximumFrameBytes);
    for (let index = 0; index < chunks.length; index++) {
      await this.sendFragment(chunks[index]!, index === chunks.length - 1, release);
    }
  }
}

export function fragmentText(channel: string, text: string, maximum: number): string[] {
  const chunks: string[] = [];
  let start = 0;
  while (start < text.length) {
    let end = Math.min(text.length, start + Math.max(1, Math.floor(maximum / 8)));
    if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1]!) && /[\uDC00-\uDFFF]/.test(text[end]!)) end--;
    assert.ok(end > start, 'fragment budget cannot contain one character');
    while (JSON.stringify({ jsonrpc: '2.0', method: 'channel/frame', params: {
      channel, data: text.slice(start, end), final: true,
    } }).length * 2 > maximum) {
      end--;
      if (end < text.length && /[\uD800-\uDBFF]/.test(text[end - 1]!) && /[\uDC00-\uDFFF]/.test(text[end]!)) end--;
      assert.ok(end > start, 'fragment envelope exceeds negotiated frame budget');
    }
    chunks.push(text.slice(start, end));
    start = end;
  }
  return chunks;
}

export async function subscribeWindow(t: TestContext, client: Connection, channel: string,
  receive: Limits = { windowBytes: 512, maximumFrameBytes: 1024 }) {
  const { result, cursor } = await offerWindow(client, channel, receive);
  if (!result.flowControl) t.skip(`Host declined windowed delivery for ${channel}: subscribe returned no flowControl (1.1.0 negotiation alone is not support)`);
  return new WindowPeer(client, channel, result.flowControl, cursor);
}

export async function windowSetup(t: TestContext, options: { longTitle?: boolean; receive?: Limits } = {}) {
  const { owner, session } = await sessionSetup(t);
  const title = options.longTitle === false ? t.uniqueId('window-title') : `${t.uniqueId('large-title')} ${'é😀\\\"'.repeat(1024)}`;
  await accepted(owner, session.sessionUri, { type: 'session/titleChanged', title });
  const client = await proposedClient(t, { initialSubscriptions: [] });
  const peer = await subscribeWindow(t, client, session.sessionUri, options.receive);
  return { owner, session, client, peer, title };
}

export function dispatchText(channel: string, clientSeq: number, title: string) {
  return JSON.stringify({ jsonrpc: '2.0', method: 'dispatchAction', params: {
    channel, clientSeq, action: { type: 'session/titleChanged', title },
  } });
}

export async function reconnectWindow(t: TestContext, client: Client, channel: string, receive: Limits,
  lastAppliedServerSeq: number) {
  await client.close();
  const connection = await t.connect({ label: 'proposed-reconnect' });
  const cursor = connection.received.length;
  const result = record(await connection.request('reconnect', {
    channel: 'ahp-root://', clientId: client.clientId, lastSeenServerSeq: lastAppliedServerSeq,
    subscriptions: [channel], windows: { items: [{ channel, flowControl: { receive }, lastAppliedServerSeq }] },
  }));
  const windows = result.windows === undefined ? [] : record(result.windows).items;
  assert.ok(Array.isArray(windows), 'reconnect windows.items is not an array');
  const kept = (windows as Record<string, unknown>[]).find((entry) => entry.channel === channel);
  if (!kept) t.skip(`Host did not keep window ${channel} on reconnect; preservation is optional`);
  const flow = kept.flowControl as FlowControl;
  assertLimits(flow.clientReceive, receive);
  assertLimits(flow.hostReceive);
  return { connection, result, kept, peer: new WindowPeer(connection, channel, flow, cursor) };
}
