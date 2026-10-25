import { spawn } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import type { ResolvedConfig } from '../config/schema.ts';
import { removeScratchDirSync } from '../config/workspace.ts';
import { RpcConnection } from '../harness/connection.ts';
import { redactEndpoint } from '../harness/redact.ts';
import type { RunEvent } from '../harness/types.ts';

export interface LaunchedHost { stop(): Promise<void> }

async function answersPing(config: ResolvedConfig, signal: AbortSignal, emit: (event: RunEvent) => void, timeoutMs: number): Promise<boolean> {
  const connection = new RpcConnection({ config, signal, id: '$startup', label: 'readiness', timeoutMs, emit });
  try {
    await connection.open();
    await connection.request('ping', { channel: 'ahp-root://' });
    return true;
  } catch {
    signal.throwIfAborted();
    return false;
  } finally { await connection.close(); }
}

async function waitBounded(promise: Promise<void>, timeoutMs: number): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([promise, new Promise<void>((resolve) => { timer = setTimeout(resolve, timeoutMs); })]);
  } finally { clearTimeout(timer); }
}

export async function launchHost(config: ResolvedConfig, signal: AbortSignal, emit: (event: RunEvent) => void): Promise<LaunchedHost | undefined> {
  if (!config.host) return undefined;
  signal.throwIfAborted();
  if (await answersPing(config, signal, emit, 1000)) {
    throw new Error(`Something is already listening at ${redactEndpoint(config.target.url)}; stop it before launching a host`);
  }
  signal.throwIfAborted();
  const child = spawn(config.host.command, {
    shell: true, detached: process.platform !== 'win32', cwd: config.host.cwd,
    env: { ...process.env, ...config.host.env }, stdio: ['ignore', 'pipe', 'pipe'],
  });
  let tail = '';
  let stopped = false;
  const startup = new AbortController();
  const readinessSignal = AbortSignal.any([signal, startup.signal]);
  const capture = (data: Buffer) => { tail = (tail + data.toString()).slice(-32_768); };
  child.stdout?.on('data', capture);
  child.stderr?.on('data', capture);
  const ended = new Promise<void>((resolve) => {
    const exited = (description: string) => {
      startup.abort(new Error(`Host process stopped (${description})`));
      resolve();
    };
    child.once('error', (error) => exited(error.message));
    child.once('exit', (code, sig) => exited(`exit ${code ?? sig}`));
  });
  const closed = new Promise<void>((resolve) => child.once('close', () => resolve()));
  const kill = (sig: NodeJS.Signals) => {
    if (!child.pid) return;
    try {
      if (process.platform === 'win32') child.kill(sig);
      else process.kill(-child.pid, sig);
    } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
  };
  const onExit = () => {
    if (stopped) return;
    try { kill('SIGKILL'); } catch { /* Shutdown must remain synchronous and best effort. */ }
    if (!config.workspace.keep) {
      try { removeScratchDirSync(config.workspace.scratchDir); } catch { /* Preserve the original exit status. */ }
    }
  };
  process.on('exit', onExit);
  let stopping: Promise<void> | undefined;
  const stop = (): Promise<void> => stopping ??= (async () => {
    kill('SIGTERM');
    await waitBounded(ended, 2000);
    // The shell may exit before its descendants; always signal the whole group.
    kill('SIGKILL');
    await waitBounded(closed, 1000);
    stopped = true;
    process.off('exit', onExit);
  })();
  const deadline = Date.now() + config.host.readyTimeoutMs;
  try {
    while (Date.now() < deadline) {
      readinessSignal.throwIfAborted();
      const ready = await answersPing(config, readinessSignal, emit, Math.min(1000, deadline - Date.now()));
      readinessSignal.throwIfAborted();
      if (ready) return { stop };
      await delay(Math.min(200, Math.max(0, deadline - Date.now())), undefined, { signal: readinessSignal });
    }
    throw new Error(`Host was not ready after ${config.host.readyTimeoutMs} ms`);
  } catch (error) {
    const reason = startup.signal.aborted ? startup.signal.reason : error;
    await stop();
    throw new Error(`${reason instanceof Error ? reason.message : String(reason)}${tail ? `\nHost output (tail):\n${tail}` : ''}`);
  }
}
