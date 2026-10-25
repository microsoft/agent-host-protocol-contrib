import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { PROTOCOL_VERSION, rootReducer, sessionReducer, chatReducer } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, InitializeResult, Snapshot, SubscribeResult } from '@microsoft/agent-host-protocol';
import { RpcConnection } from './connection.ts';
import type { ConnectionOptions } from './connection.ts';
import type { Client, ClientOptions, JsonRpcMessage, LooseStateAction, WaitOptions } from './types.ts';

const normalized = (value: unknown): unknown => value === undefined ? undefined : JSON.parse(JSON.stringify(value));

export class ProtocolClient extends RpcConnection implements Client {
  readonly clientId: string;
  initializeResult!: InitializeResult;
  private seq = 0;
  private clientSeq = 0;
  private mirrors = new Map<string, { state: unknown; fromSeq: number }>();
  private envelopes: ActionEnvelope[] = [];

  constructor(options: ConnectionOptions, clientId = `conformance-${randomUUID()}`) {
    super(options);
    this.clientId = clientId;
    this.observers.add((message) => this.mirror(message));
  }

  async initialize(options: ClientOptions = {}): Promise<this> {
    this.initializeResult = await this.request<InitializeResult>('initialize', {
      channel: 'ahp-root://', clientId: this.clientId,
      clientInfo: { name: 'ahp-conformance', version: '0.1.0' },
      protocolVersions: options.protocolVersions ?? [PROTOCOL_VERSION],
      initialSubscriptions: options.initialSubscriptions ?? ['ahp-root://'],
    });
    this.seq = Math.max(this.seq, this.initializeResult.serverSeq);
    if (options.authenticate !== false) {
      for (const token of this.options.config.auth.tokens) await this.request('authenticate', { channel: 'ahp-root://', ...token });
    }
    return this;
  }

  get lastServerSeq(): number { return this.seq; }

  private snapshot(snapshot: Snapshot): void {
    this.seq = Math.max(this.seq, snapshot.fromSeq);
    this.mirrors.set(snapshot.resource, { state: snapshot.state, fromSeq: snapshot.fromSeq });
    for (const envelope of this.envelopes) {
      if (envelope.channel === snapshot.resource && envelope.serverSeq > snapshot.fromSeq) this.apply(envelope);
    }
  }

  private mirror(message: JsonRpcMessage): void {
    // Install response snapshots synchronously, before a subsequent frame is reduced.
    if (message.result && typeof message.result === 'object') {
      const result = message.result as { snapshots?: Snapshot[]; snapshot?: Snapshot; serverSeq?: number };
      if (typeof result.serverSeq === 'number') this.seq = Math.max(this.seq, result.serverSeq);
      if (result.snapshots) for (const snapshot of result.snapshots) this.snapshot(snapshot);
      if (result.snapshot) this.snapshot(result.snapshot);
    }
    if (message.method === 'action' && message.id === undefined) {
      const envelope = message.params as ActionEnvelope;
      this.seq = Math.max(this.seq, envelope.serverSeq);
      this.envelopes.push(envelope);
      this.apply(envelope);
    }
  }

  private apply(envelope: ActionEnvelope): void {
    const mirror = this.mirrors.get(envelope.channel);
    if (!mirror || envelope.serverSeq <= mirror.fromSeq || envelope.rejectionReason) return;
    if (envelope.channel === 'ahp-root://') {
      mirror.state = rootReducer(mirror.state as Parameters<typeof rootReducer>[0], envelope.action as Parameters<typeof rootReducer>[1]);
    } else if (envelope.channel.startsWith('ahp-session:')) {
      mirror.state = sessionReducer(mirror.state as Parameters<typeof sessionReducer>[0], envelope.action as Parameters<typeof sessionReducer>[1]);
    } else if (envelope.channel.startsWith('ahp-chat:')) {
      mirror.state = chatReducer(mirror.state as Parameters<typeof chatReducer>[0], envelope.action as Parameters<typeof chatReducer>[1]);
    }
    mirror.fromSeq = envelope.serverSeq;
  }

  async subscribe(uri: string): Promise<Snapshot> {
    const result = await this.request<SubscribeResult>('subscribe', { channel: uri });
    // SDK 1.0's Snapshot union omits the protocol's stateless-channel shape.
    return result.snapshot ?? { resource: uri, state: {} as Snapshot['state'], fromSeq: this.seq };
  }

  async unsubscribe(uri: string): Promise<void> {
    this.notify('unsubscribe', { channel: uri });
    this.mirrors.delete(uri);
  }

  state<S = unknown>(uri: string): S | undefined { return this.mirrors.get(uri)?.state as S | undefined; }

  dispatch(channel: string, action: LooseStateAction): number {
    const clientSeq = ++this.clientSeq;
    this.notify('dispatchAction', { channel, action, clientSeq });
    return clientSeq;
  }

  actions(channel?: string): readonly ActionEnvelope[] {
    return channel ? this.envelopes.filter((envelope) => envelope.channel === channel) : this.envelopes;
  }

  async waitForAction(channel: string, predicate: (envelope: ActionEnvelope) => boolean, options?: WaitOptions): Promise<ActionEnvelope> {
    return this.waitForNotification<ActionEnvelope>('action', (envelope) => envelope.channel === channel && predicate(envelope), options);
  }

  async waitForState<S = unknown>(uri: string, predicate: (state: S) => boolean, options?: WaitOptions): Promise<S> {
    const current = this.state<S>(uri);
    if (current !== undefined && predicate(current)) return current;
    let observer: (message: JsonRpcMessage) => void;
    const promise = new Promise<S>((resolve, reject) => {
      observer = () => {
        try {
          const state = this.state<S>(uri);
          if (state !== undefined && predicate(state)) resolve(state);
        } catch (error) { reject(error); }
      };
      this.observers.add(observer);
      void this.closed.then(({ code }) => reject(new Error(`Connection closed while waiting for state (${code})`)));
    });
    try { return await this.bounded(promise, options?.timeoutMs, `Waiting for state ${uri}`); }
    finally { this.observers.delete(observer!); }
  }

  async assertMirrorMatchesSnapshot(uri: string): Promise<void> {
    const fresh = new ProtocolClient({ ...this.options, label: `${this.label}-snapshot` });
    try {
      await fresh.open();
      await fresh.initialize({ initialSubscriptions: [] });
      const snapshot = await fresh.subscribe(uri);
      assert.deepEqual(normalized(this.state(uri)), normalized(snapshot.state), `Mirror differs from fresh snapshot for ${uri}`);
    } finally { await fresh.close(); }
  }
}
