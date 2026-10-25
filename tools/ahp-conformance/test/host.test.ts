import assert from 'node:assert/strict';
import { once } from 'node:events';
import test from 'node:test';
import { WebSocketServer } from 'ws';
import type { ResolvedConfig } from '../src/config/schema.ts';
import { launchHost } from '../src/host/index.ts';

function config(url: string, command: string): ResolvedConfig {
  return {
    configPath: '/config.yml', target: { url, headers: {} }, auth: { tokens: [] }, agent: { catalogTimeoutMs: 30_000 },
    model: { enabled: false, deterministic: false }, quirks: { lazySessionInit: false },
    workspace: { scratchDir: '/unowned', sharedFilesystem: true, keep: false },
    run: { concurrency: 1, timeoutMs: 30_000, include: [], exclude: [], proposed: false },
    report: { reporter: 'cli', open: false },
    host: { command, cwd: process.cwd(), env: {}, readyTimeoutMs: 10_000 },
  };
}

async function endpoint(): Promise<{ server: WebSocketServer; url: string }> {
  const server = new WebSocketServer({ host: '127.0.0.1', port: 0 });
  await once(server, 'listening');
  const address = server.address();
  assert.ok(address && typeof address !== 'string');
  return { server, url: `ws://127.0.0.1:${address.port}/ahp` };
}

async function close(server: WebSocketServer): Promise<void> {
  await new Promise<void>((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
}

test('host startup refuses an endpoint already answering pings without spawning', async () => {
  const { server, url } = await endpoint();
  server.on('connection', (socket) => socket.on('message', (data) => {
    const request = JSON.parse(data.toString()) as { id: number; method: string };
    assert.equal(request.method, 'ping');
    socket.send(JSON.stringify({ jsonrpc: '2.0', id: request.id, result: {} }));
  }));
  try {
    await assert.rejects(launchHost(config(url, 'this-command-must-not-run'), new AbortController().signal, () => {}), /Something is already listening at/);
  } finally { await close(server); }
});

test('a child exiting before readiness fails immediately and includes its output tail', { timeout: 5000 }, async () => {
  const { server, url } = await endpoint();
  await close(server);
  const command = `"${process.execPath}" -e 'console.error("host-exit-marker"); process.exit(7)'`;
  await assert.rejects(launchHost(config(url, command), new AbortController().signal, () => {}), (error: unknown) => {
    assert.ok(error instanceof Error);
    assert.match(error.message, /Host process stopped \(exit 7\)/);
    assert.match(error.message, /host-exit-marker/);
    return true;
  });
});
