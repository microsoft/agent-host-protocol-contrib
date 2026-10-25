import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { resolveConfig } from '../src/config/load.ts';
import type { ResolvedConfig } from '../src/config/schema.ts';
import { removeScratchDir } from '../src/config/workspace.ts';
import { configuredSecrets, redact, redactEndpoint, redactFrame } from '../src/harness/redact.ts';

function fixture(): ResolvedConfig {
  return {
    configPath: '/config.yml',
    agent: { catalogTimeoutMs: 30_000 },
    model: { enabled: true, deterministic: false }, quirks: { lazySessionInit: false },
    workspace: { scratchDir: '/scratch', sharedFilesystem: true, keep: false },
    run: { concurrency: 4, timeoutMs: 30_000, include: [], exclude: [], proposed: false },
    report: { reporter: 'cli', open: false },
    target: { url: 'ws://example.com/ahp', headers: {} },
    auth: { tokens: [] },
  };
}

test('authenticate tokens are structurally removed, including short and unconfigured tokens', () => {
  for (const token of ['dynamic-token', 'abc']) {
    const frame = { jsonrpc: '2.0', id: 1, method: 'authenticate', params: { channel: 'ahp-root://', resource: 'example', token } };
    const safe = redactFrame(JSON.stringify(frame), []);
    assert.equal((safe as typeof frame).params.token, '[REDACTED]');
    assert.equal(frame.params.token, token);
  }
});

test('configured secrets are redacted in nested values and malformed raw frames', () => {
  assert.deepEqual(redact({ nested: ['prefix private-key', { text: 'private-key again' }] }, ['private-key']),
    { nested: ['prefix [REDACTED]', { text: '[REDACTED] again' }] });
  assert.equal(redactFrame('not-json private-key', ['private-key']), 'not-json [REDACTED]');
  assert.equal(redactFrame('{"method":"authenticate","params":{"token":"unknown-token",oops', []),
    '{"method":"authenticate","params":{"token":"[REDACTED]",oops');
});

test('reported endpoint removes userinfo and every query value, but preserves the path', () => {
  const endpoint = new URL('wss://example.com/ahp?token=secret&other=another&token=second');
  endpoint.username = 'sample-user';
  endpoint.password = 'password-secret';
  const url = new URL(redactEndpoint(endpoint.href));
  assert.equal(url.username, 'REDACTED');
  assert.equal(url.password, 'REDACTED');
  assert.equal(url.pathname, '/ahp');
  assert.deepEqual([...url.searchParams.values()], ['REDACTED', 'REDACTED']);
});

test('short header values and literal URL query values do not corrupt protocol frames', () => {
  const config = fixture();
  config.target.url = 'ws://example.com/ahp?v=1&setting=public-query-value';
  config.target.headers = { 'X-Client': 'ahp' };
  const secrets = configuredSecrets(config);
  assert.deepEqual(secrets, []);
  const frame = { protocolVersion: '1.0.0', channel: 'ahp-session:/example', setting: 'public-query-value' };
  assert.deepEqual(redact(frame, secrets), frame);
  assert.deepEqual(redact(frame, ['1', 'ahp']), frame);
});

test('all header, explicit query, auth-token, and userinfo secret forms are collected', async () => {
  const parent = join(tmpdir(), `ahp-conformance-redact-${randomUUID()}`);
  await mkdir(parent, { recursive: true });
  let config: ResolvedConfig | undefined;
  try {
    const endpoint = new URL('ws://example.com/ahp?embedded=public-query-value');
    endpoint.username = 'sample-user';
    endpoint.password = 'password-secret';
    config = await resolveConfig({
      target: {
        url: endpoint.href,
        headers: { 'X-Plain': 'plain-header-secret', 'X-Wrapped': { value: 'wrapped-header-secret' }, 'X-Client': 'ahp' },
        query: { plain: 'plain-query-secret', wrapped: { value: 'wrapped-query-secret' }, short: '1' },
      },
      auth: { tokens: [{ resource: 'example', token: 'auth-secret' }] },
      workspace: { scratchDir: parent },
    }, resolve('test/redaction-config.yml'));
    const secrets = configuredSecrets(config);
    for (const secret of ['sample-user', 'password-secret', 'plain-header-secret', 'wrapped-header-secret', 'plain-query-secret', 'wrapped-query-secret', 'auth-secret']) {
      assert.ok(secrets.includes(secret), `Expected configured secret: ${secret}`);
    }
    for (const value of ['public-query-value', 'ahp', '1', '']) assert.ok(!secrets.includes(value));
    assert.equal(redact('plain-query-secret wrapped-query-secret', secrets), '[REDACTED] [REDACTED]');
  } finally {
    if (config) await removeScratchDir(config.workspace.scratchDir);
    await rm(parent, { recursive: true, force: true });
  }
});
