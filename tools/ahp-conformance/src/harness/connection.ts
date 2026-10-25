import { JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import WebSocket from 'ws';
import type { ResolvedConfig } from '../config/schema.ts';
import { configuredSecrets, redactFrame } from './redact.ts';
import { RpcError } from './types.ts';
import type { Connection, JsonRpcMessage, ReceivedFrame, RunEvent, WaitOptions } from './types.ts';

export interface ConnectionOptions {
  config: ResolvedConfig;
  id: string;
  label: string;
  signal?: AbortSignal;
  timeoutMs?: number;
  emit: (event: RunEvent) => void;
}

interface Waiter {
  predicate: (message: JsonRpcMessage) => boolean;
  resolve: (message: JsonRpcMessage) => void;
  reject: (error: unknown) => void;
}
interface Pending {
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
}

export class RpcConnection implements Connection {
  readonly label: string;
  readonly received: JsonRpcMessage[] = [];
  readonly receivedFrames: ReceivedFrame[] = [];
  readonly closed: Promise<{ code: number; reason: string }>;
  protected readonly options: ConnectionOptions;
  protected readonly observers = new Set<(message: JsonRpcMessage) => void>();
  private socket?: WebSocket;
  private nextId = 0;
  private pending = new Map<number, Pending>();
  private waiters = new Set<Waiter>();
  private handlers = new Map<string, (params: unknown) => unknown | Promise<unknown>>();
  private resolveClosed!: (value: { code: number; reason: string }) => void;
  private failure?: Error;
  private secrets: string[];
  private abortListener?: () => void;

  constructor(options: ConnectionOptions) {
    this.options = options;
    this.label = options.label;
    this.secrets = configuredSecrets(options.config);
    this.closed = new Promise((resolve) => { this.resolveClosed = resolve; });
  }

  async open(): Promise<this> {
    this.options.signal?.throwIfAborted();
    const socket = new WebSocket(this.options.config.target.url, { headers: this.options.config.target.headers });
    this.socket = socket;
    socket.on('message', (data) => this.receive(data.toString()));
    socket.on('error', (error) => this.fail(error));
    const abort = () => {
      const error = this.options.signal?.reason ?? new Error('Run cancelled');
      for (const pending of this.pending.values()) pending.reject(error);
      for (const waiter of this.waiters) waiter.reject(error);
      if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
    };
    this.abortListener = abort;
    this.options.signal?.addEventListener('abort', abort, { once: true });
    socket.on('close', (code, reason) => {
      this.options.signal?.removeEventListener('abort', abort);
      this.fail(new Error(`Connection ${this.label} closed (${code}${reason.length ? `: ${reason}` : ''})`));
      this.resolveClosed({ code, reason: reason.toString() });
    });
    try {
      await this.bounded(new Promise<void>((resolve, reject) => {
        const done = () => { socket.off('open', opened); socket.off('error', errored); socket.off('close', closed); };
        const opened = () => { done(); resolve(); };
        const errored = (error: Error) => { done(); reject(error); };
        const closed = () => { done(); reject(new Error('WebSocket closed before opening')); };
        socket.once('open', opened);
        socket.once('error', errored);
        socket.once('close', closed);
      }), this.options.timeoutMs, 'WebSocket connection');
    } catch (error) {
      socket.terminate();
      throw error;
    }
    return this;
  }

  protected bounded<T>(promise: Promise<T>, timeoutMs = this.options.timeoutMs ?? this.options.config.run.timeoutMs, label = 'Operation'): Promise<T> {
    return new Promise((resolve, reject) => {
      const finish = (error: unknown, value?: T) => {
        clearTimeout(timer);
        this.options.signal?.removeEventListener('abort', aborted);
        if (error !== undefined) reject(error);
        else resolve(value as T);
      };
      const aborted = () => finish(this.options.signal?.reason ?? new Error('Run cancelled'));
      const timer = setTimeout(() => finish(new Error(`${label} timed out after ${timeoutMs} ms`)), timeoutMs);
      promise.then((value) => finish(undefined, value), (error) => finish(error));
      this.options.signal?.addEventListener('abort', aborted, { once: true });
      if (this.options.signal?.aborted) aborted();
    });
  }

  request<R = unknown>(method: string, params?: unknown, options?: WaitOptions): Promise<R> {
    const id = ++this.nextId;
    const promise = new Promise<unknown>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      try { this.send({ jsonrpc: '2.0', id, method, ...(params !== undefined && { params }) }); }
      catch (error) { this.pending.delete(id); reject(error); }
    });
    return this.bounded(promise, options?.timeoutMs, method).finally(() => this.pending.delete(id)) as Promise<R>;
  }

  notify(method: string, params?: unknown): void {
    this.send({ jsonrpc: '2.0', method, ...(params !== undefined && { params }) });
  }

  private send(message: JsonRpcMessage): void { this.sendRaw(JSON.stringify(message)); }

  sendRaw(text: string): void {
    this.options.signal?.throwIfAborted();
    if (this.failure) throw this.failure;
    if (this.socket?.readyState !== WebSocket.OPEN) throw new Error('WebSocket is not open');
    try {
      const message = JSON.parse(text) as { method?: string; params?: { token?: unknown } };
      if (message?.method === 'authenticate' && typeof message.params?.token === 'string' && message.params.token.length >= 6) {
        this.secrets.push(message.params.token, encodeURIComponent(message.params.token));
      }
    } catch { /* Raw malformed frames still belong in the transcript. */ }
    this.frame('send', text);
    this.socket.send(text);
  }

  private frame(direction: 'send' | 'recv', text: string): void {
    this.options.emit({ type: 'test-frame', at: new Date().toISOString(), id: this.options.id,
      frame: { connection: this.label, direction, message: redactFrame(text, this.secrets) } });
  }

  private receive(text: string): void {
    this.frame('recv', text);
    const frame: ReceivedFrame = { text, utf16Bytes: text.length * 2 };
    this.receivedFrames.push(frame);
    let message: JsonRpcMessage;
    try {
      const parsed: unknown = JSON.parse(text);
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return;
      message = parsed as JsonRpcMessage;
    } catch { return; }
    frame.message = message;
    this.received.push(message);
    try { for (const observer of this.observers) observer(message); }
    catch (error) { this.fail(error instanceof Error ? error : new Error(String(error))); return; }
    if (typeof message.method === 'string' && message.id !== undefined) {
      void this.handleRequest(message);
    } else if (typeof message.id === 'number') {
      const pending = this.pending.get(message.id);
      if (pending) {
        this.pending.delete(message.id);
        if (message.error && typeof message.error === 'object') {
          const error = message.error as { code: number; message: string; data?: unknown };
          pending.reject(new RpcError(error.code, error.message, error.data));
        } else pending.resolve(message.result);
      }
    }
    for (const waiter of [...this.waiters]) {
      try { if (waiter.predicate(message)) { this.waiters.delete(waiter); waiter.resolve(message); } }
      catch (error) { this.waiters.delete(waiter); waiter.reject(error); }
    }
  }

  private async handleRequest(message: JsonRpcMessage): Promise<void> {
    try {
      const handler = this.handlers.get(message.method as string);
      if (!handler) throw new RpcError(JsonRpcErrorCodes.MethodNotFound, `Method not found: ${message.method}`);
      const result = await handler(message.params);
      this.send({ jsonrpc: '2.0', id: message.id, result: result ?? null });
    } catch (error) {
      try {
        this.send({ jsonrpc: '2.0', id: message.id, error: {
          code: error instanceof RpcError ? error.code : JsonRpcErrorCodes.InternalError,
          message: error instanceof Error ? error.message : String(error),
          ...(error instanceof RpcError && error.data !== undefined && { data: error.data }),
        } });
      } catch { /* The peer may close while its request handler is running. */ }
    }
  }

  onRequest(method: string, handler: (params: unknown) => unknown | Promise<unknown>): void { this.handlers.set(method, handler); }

  async waitForMessage(predicate: (message: JsonRpcMessage) => boolean, options?: WaitOptions): Promise<JsonRpcMessage> {
    const buffered = this.received.find(predicate);
    if (buffered) return buffered;
    if (this.failure) throw this.failure;
    let waiter: Waiter;
    const promise = new Promise<JsonRpcMessage>((resolve, reject) => {
      waiter = { predicate, resolve, reject };
      this.waiters.add(waiter);
    });
    try { return await this.bounded(promise, options?.timeoutMs, 'Waiting for message'); }
    finally { this.waiters.delete(waiter!); }
  }

  async waitForNotification<P = unknown>(method: string, predicate: (params: P) => boolean = () => true, options?: WaitOptions): Promise<P> {
    const message = await this.waitForMessage((entry) => entry.id === undefined && entry.method === method && predicate(entry.params as P), options);
    return message.params as P;
  }

  async expectNoNotification<P = unknown>(method: string, predicate: (params: P) => boolean, durationMs: number): Promise<void> {
    let observer: (message: JsonRpcMessage) => void;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const promise = new Promise<void>((resolve, reject) => {
      observer = (entry) => {
        try {
          if (entry.id === undefined && entry.method === method && predicate(entry.params as P)) reject(new Error(`Unexpected ${method} notification`));
        } catch (error) { reject(error); }
      };
      this.observers.add(observer);
      timer = setTimeout(resolve, durationMs);
    });
    try { await this.bounded(promise, durationMs + 1000, `Observing ${method}`); }
    finally { clearTimeout(timer); this.observers.delete(observer!); }
  }

  private fail(error: Error): void {
    this.failure ??= error;
    for (const pending of this.pending.values()) pending.reject(this.failure);
    this.pending.clear();
    for (const waiter of this.waiters) waiter.reject(this.failure);
    this.waiters.clear();
  }

  beginCleanup(): void {
    if (this.abortListener) this.options.signal?.removeEventListener('abort', this.abortListener);
    this.options.signal = undefined;
  }

  async close(): Promise<void> {
    const socket = this.socket;
    if (!socket || socket.readyState === WebSocket.CLOSED) return;
    if (socket.readyState === WebSocket.CONNECTING) socket.terminate();
    else socket.close();
    const timer = setTimeout(() => socket.terminate(), 1000);
    try { await this.closed; }
    finally { clearTimeout(timer); }
  }
}
