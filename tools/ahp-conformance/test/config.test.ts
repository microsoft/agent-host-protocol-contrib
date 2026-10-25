import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, isAbsolute, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { parse } from 'yaml';
import { configSchema } from '../src/config/schema.ts';
import { resolveConfig } from '../src/config/load.ts';
import { resolveSecret } from '../src/config/secrets.ts';
import { configTemplate, writeTemplate } from '../src/config/template.ts';
import { createScratchDir, findGitWorkTree, removeScratchDir } from '../src/config/workspace.ts';

const target = { url: 'ws://127.0.0.1:8787/ahp' };

test('configuration applies defaults', () => {
  const value = configSchema.parse({ target });
  assert.deepEqual(value.run, { concurrency: 4, timeoutMs: 30_000, include: [], exclude: [], proposed: false });
  assert.deepEqual(value.model, { enabled: true, deterministic: false });
  assert.deepEqual(value.workspace, { sharedFilesystem: true, keep: false });
  assert.deepEqual(value.auth.tokens, []);
  assert.deepEqual(value.target.headers, {});
  assert.equal(value.report.reporter, 'auto');
  assert.equal(value.report.open, true);
  assert.equal(value.report.htmlFile, 'ahp-conformance-report.html');
  assert.equal(configSchema.parse({ target, host: { command: 'node server.js' } }).host?.readyTimeoutMs, 120_000);
});

test('configuration rejects invalid URLs, unknown keys, invalid integers, and ambiguous secrets', () => {
  for (const input of [
    {}, { target: { url: 'https://example.com' } }, { target: { url: 'not a URL' } },
    { target, run: { concurrency: 0 } }, { target, run: { timeoutMs: 1.5 } },
    { target, model: { unknown: true } }, { target, extra: true },
    { target: { ...target, headers: { Authorization: { env: 'TOKEN', value: 'oops' } } } },
  ]) assert.equal(configSchema.safeParse(input).success, false);
});

test('template is a valid, fully defaulted YAML configuration', () => {
  const template = configTemplate(resolve('ahp-conformance.yml'));
  assert.match(template.split('\n')[0], /^# yaml-language-server: \$schema=.*conformance\.schema\.json$/);
  assert.match(template, /\$\{scratchDir\}/);
  const value = configSchema.parse(parse(template));
  assert.equal(value.target.url, target.url);
  assert.deepEqual(value.run.include, []);
  assert.equal(value.workspace.keep, false);
});

test('secret sources resolve literal, value, env, and trimmed command stdout', async () => {
  assert.equal(await resolveSecret('literal'), 'literal');
  assert.equal(await resolveSecret({ value: 'wrapped' }), 'wrapped');
  const name = `AHP_CONFORMANCE_UNIT_${randomUUID().replaceAll('-', '_')}`;
  process.env[name] = 'environment';
  try { assert.equal(await resolveSecret({ env: name }), 'environment'); }
  finally { delete process.env[name]; }
  await assert.rejects(resolveSecret({ env: name }), /is not set/);
  assert.equal(await resolveSecret({ command: `"${process.execPath}" -e 'process.stdout.write("  command-value\\n")'` }), 'command-value');
  await assert.rejects(resolveSecret({ command: `"${process.execPath}" -e 'process.exit(7)'` }), /Secret command failed/);
});

test('resolved paths, query secrets, overrides, and scratch realpath', async () => {
  const dir = join(tmpdir(), `ahp-conformance-config-${randomUUID()}`);
  await mkdir(dir, { recursive: true });
  try {
    const config = await resolveConfig({
      target: { url: `${target.url}?existing=yes`, headers: { Authorization: { value: 'secret' } }, query: { token: 'a b' } },
      workspace: { scratchDir: './scratch' },
      host: { command: 'node server.js ${scratchDir}', cwd: '../..', env: { WORKSPACE: '${scratchDir}' } },
    }, `${dir}/config.yml`, { reporter: 'cli', open: false, jsonFile: 'results.json' });
    assert.equal(dirname(config.workspace.scratchDir), await realpath(`${dir}/scratch`));
    assert.equal(config.workspace.scratchDir, await realpath(config.workspace.scratchDir));
    assert.match(basename(config.workspace.scratchDir), /^run-\d+-/);
    assert.equal(config.host?.cwd, resolve(dir, '../..'));
    assert.ok(config.host?.command.endsWith(config.workspace.scratchDir));
    assert.equal(config.host?.env.WORKSPACE, config.workspace.scratchDir);
    assert.equal(new URL(config.target.url).searchParams.get('token'), 'a b');
    assert.equal(new URL(config.target.url).searchParams.get('existing'), 'yes');
    assert.deepEqual(config.target.headers, { Authorization: 'secret' });
    assert.equal(config.report.open, false);
    assert.equal(config.report.jsonFile, resolve('results.json'));
    assert.equal(config.report.htmlFile, resolve(dir, 'ahp-conformance-report.html'));
  } finally { await rm(dir, { recursive: true, force: true }); }
});

test('init refuses to overwrite an existing configuration', async () => {
  const dir = resolve(`test/.init-${randomUUID()}`);
  await mkdir(dir, { recursive: true });
  try {
    await writeTemplate(`${dir}/config.yml`);
    await assert.rejects(writeTemplate(`${dir}/config.yml`), { code: 'EEXIST' });
  } finally { await rm(dir, { recursive: true, force: true }); }
});


test('scratch cleanup preserves configured parents and directories it did not create', async () => {
  const parent = join(tmpdir(), `ahp-conformance-scratch-${randomUUID()}`);
  await mkdir(parent, { recursive: true });
  await writeFile(`${parent}/existing.txt`, 'preserve me');
  try {
    const first = await createScratchDir(parent, process.cwd());
    const second = await createScratchDir(parent, process.cwd());
    assert.notEqual(first, second);
    assert.equal(dirname(first), await realpath(parent));
    await removeScratchDir(first);
    await removeScratchDir(second);
    await assert.rejects(realpath(first), { code: 'ENOENT' });
    await removeScratchDir(parent);
    assert.equal(await readFile(`${parent}/existing.txt`, 'utf8'), 'preserve me');
  } finally { await rm(parent, { recursive: true, force: true }); }
});

test('scratch directories inside a git repository are refused', async () => {
  const repository = findGitWorkTree(process.cwd());
  assert.ok(repository, 'unit tests run inside the repository checkout');
  await assert.rejects(createScratchDir(resolve(`test/.scratch-${randomUUID()}`), process.cwd()), /inside the git repository/);
  assert.equal(findGitWorkTree(await realpath(tmpdir())), undefined);
});

test('schema headers are relative inside the repository and absolute outside it', () => {
  const schemaPath = fileURLToPath(new URL('../conformance.schema.json', import.meta.url));
  const repositoryRoot = fileURLToPath(new URL('../../../', import.meta.url));
  const header = (path: string) => configTemplate(path).split('\n')[0].split('$schema=')[1];
  assert.equal(isAbsolute(header(resolve(repositoryRoot, 'config.yml'))), false);
  assert.equal(header(resolve(repositoryRoot, '..', 'external-config.yml')), schemaPath.replaceAll('\\', '/'));
});
