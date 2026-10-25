import { randomUUID } from 'node:crypto';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import type { CreateSessionParams } from '@microsoft/agent-host-protocol';
import { RpcConnection } from './connection.ts';
import { ProtocolClient } from './client.ts';
import { ProtocolSession } from './session.ts';
import type { Client, ClientOptions, Connection, HostProfile, RunEvent, SessionHandle, TestContext } from './types.ts';

export class TestSkipped extends Error {}

export class Context implements TestContext {
  readonly id: string;
  readonly profile: HostProfile;
  readonly config: HostProfile['config'];
  readonly signal: AbortSignal;
  readonly scratchDir: string;
  private cleanups: (() => unknown)[] = [];
  private emit: (event: RunEvent) => void;
  private connectionNumber = 0;
  private connections: RpcConnection[] = [];
  private cleaning = false;
  private timeoutMs: number;

  constructor(options: { id: string; profile: HostProfile; signal: AbortSignal; scratchDir: string; timeoutMs?: number; emit: (event: RunEvent) => void }) {
    this.id = options.id;
    this.profile = options.profile;
    this.config = options.profile.config;
    this.signal = options.signal;
    this.scratchDir = options.scratchDir;
    this.emit = options.emit;
    this.timeoutMs = options.timeoutMs ?? this.config.run.timeoutMs;
  }

  scratchUri(relativePath = ''): string { return pathToFileURL(resolve(this.scratchDir, relativePath)).href; }
  uniqueId(prefix = 'conformance'): string { return `${prefix}-${randomUUID()}`; }
  log(message: string, data?: unknown): void { this.emit({ type: 'test-log', at: new Date().toISOString(), id: this.id, message, ...(data !== undefined && { data }) }); }
  defer(cleanup: () => unknown): void { this.cleanups.push(cleanup); }
  skip(reason: string): never { throw new TestSkipped(reason); }

  private connectionOptions(label?: string) {
    this.signal.throwIfAborted();
    if (this.cleaning) throw new Error('Test has finished');
    return { config: this.config, id: this.id, label: label ?? `connection-${++this.connectionNumber}`, signal: this.signal, timeoutMs: this.timeoutMs, emit: this.emit };
  }

  async connect(options?: { label?: string }): Promise<Connection> {
    const connection = new RpcConnection(this.connectionOptions(options?.label));
    this.connections.push(connection);
    this.defer(() => connection.close());
    return connection.open();
  }

  async client(options: ClientOptions = {}): Promise<Client> {
    const client = new ProtocolClient(this.connectionOptions(options.label), options.clientId);
    this.connections.push(client);
    this.defer(() => client.close());
    await client.open();
    return client.initialize(options);
  }

  async session(client: Client, overrides?: Partial<CreateSessionParams>): Promise<SessionHandle> {
    const session = await ProtocolSession.create(this, client, overrides);
    return session;
  }

  async cleanup(graceMs = 3000): Promise<unknown[]> {
    this.cleaning = true;
    for (const connection of this.connections) connection.beginCleanup();
    const errors: unknown[] = [];
    let timer: ReturnType<typeof setTimeout> | undefined;
    const work = async () => {
      for (const cleanup of this.cleanups.reverse()) {
        try { await cleanup(); }
        catch (error) { errors.push(error); }
      }
      this.cleanups = [];
    };
    try {
      await Promise.race([work(), new Promise<void>((resolve) => {
        timer = setTimeout(() => { errors.push(new Error(`Cleanup timed out after ${graceMs} ms`)); resolve(); }, graceMs);
      })]);
    } finally {
      clearTimeout(timer);
      await Promise.allSettled(this.connections.map((connection) => connection.close()));
    }
    return errors;
  }
}
