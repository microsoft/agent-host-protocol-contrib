import assert from 'node:assert/strict';
import test from 'node:test';
import { requires } from '../src/harness/requirements.ts';
import type { HostProfile } from '../src/harness/types.ts';
import { skipReason } from '../src/runner/requirements.ts';

function profile(): HostProfile {
  const agent = { provider: 'sample', displayName: 'Sample', description: '', models: [] };
  return {
    initialize: { protocolVersion: '1.0.0', serverSeq: 0, snapshots: [] }, root: { agents: [agent] }, agent,
    config: {
      configPath: '/config.yml', target: { url: 'ws://localhost/ahp', headers: {} }, auth: { tokens: [] }, agent: { catalogTimeoutMs: 30_000 },
      model: { enabled: true, deterministic: false }, quirks: { lazySessionInit: false }, workspace: { scratchDir: '/scratch', sharedFilesystem: true, keep: false },
      run: { concurrency: 4, timeoutMs: 30_000, include: [], exclude: [], proposed: false }, report: { reporter: 'cli', open: false },
    },
  };
}

test('capability gating skips only absent advertised features', () => {
  const host = profile();
  assert.equal(skipReason({ requires: [requires.multipleChats] }, host), 'Host does not advertise multipleChats');
  host.agent.capabilities = { multipleChats: {} };
  assert.equal(skipReason({ requires: [requires.multipleChats] }, host), undefined);
  assert.equal(skipReason({ requires: [requires.fork] }, host), 'Host does not advertise multipleChats.fork');
  host.agent.capabilities.multipleChats!.fork = true;
  assert.equal(skipReason({ requires: [requires.fork] }, host), undefined);
});

test('config gating uses explicit disabled reasons and returns the first unmet requirement', () => {
  const host = profile();
  assert.equal(skipReason({}, host), undefined);
  assert.equal(skipReason({ requires: [requires.model] }, host), undefined);
  host.config.model.enabled = false;
  assert.equal(skipReason({ requires: [requires.model, requires.multipleChats] }, host), 'Disabled in config: model');
  assert.equal(skipReason({ requires: [requires.authTokens] }, host), 'Disabled in config: auth.tokens');
  assert.equal(skipReason({ requires: [requires.deterministicModel] }, host), 'Disabled in config: model.deterministic');
  host.config.model.enabled = true;
  host.config.model.deterministic = true;
  assert.equal(skipReason({ requires: [requires.deterministicModel] }, host), undefined);
});
