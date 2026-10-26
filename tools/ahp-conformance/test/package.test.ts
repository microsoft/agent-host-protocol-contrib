import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { parse } from 'yaml';

const execute = promisify(execFile);
const directory = fileURLToPath(new URL('..', import.meta.url));

test('the npm artifact installs and runs without TypeScript or development dependencies', { timeout: 60_000 }, async (t) => {
  const root = await mkdtemp(join(tmpdir(), 'ahp-conformance-package-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const npm = process.env.npm_execpath;
  assert.ok(npm, 'Run the package tests with npm test');
  const packed = await execute(process.execPath, [
    npm, 'pack', '--json', '--ignore-scripts', '--pack-destination', root,
  ], { cwd: directory, timeout: 30_000 });
  const [artifact] = JSON.parse(packed.stdout);
  assert.equal(artifact.name, '@microsoft/agent-host-protocol-conformance');
  const files = artifact.files.map((file: { path: string }) => file.path);
  for (const file of [
    'dist/cli.js', 'dist/report/html/index.js', 'dist/report/html/page.html',
    'dist/report/html/page.css', 'dist/report/html/page.js', 'conformance.schema.json',
    'LICENSE', 'CHANGELOG.md', 'examples/advanced-workspace-host.yml',
  ]) assert.ok(files.includes(file), `Missing published file: ${file}`);
  assert.ok(files.every((file: string) => !file.endsWith('.ts') && !/^(src|test|scripts)\//.test(file)));
  await execute(process.execPath, [
    npm, 'install', '--prefix', root, '--omit=dev', '--ignore-scripts', '--no-audit', '--no-fund',
    join(root, artifact.filename),
  ], { timeout: 30_000 });
  const installed = join(root, 'node_modules', '@microsoft', 'agent-host-protocol-conformance');
  const manifest = JSON.parse(await readFile(join(installed, 'package.json'), 'utf8'));
  assert.deepEqual(manifest.bin, { 'ahp-conformance': './dist/cli.js' });
  await assert.rejects(readFile(join(root, 'node_modules', 'typescript', 'package.json')), { code: 'ENOENT' });
  const invoke = (args: string[]) => execute(process.execPath, [
    npm, 'exec', '--offline', '--', 'ahp-conformance', ...args,
  ], { cwd: root, timeout: 10_000 });
  assert.match((await invoke(['--help'])).stdout, /Usage: ahp-conformance/);
  assert.match((await invoke(['--list', '--filter', 'transport/**'])).stdout, /transport\//);
  await invoke(['--init']);
  const config = await readFile(join(root, 'ahp-conformance.yml'), 'utf8');
  assert.equal(parse(config).target.url, 'ws://127.0.0.1:8787/ahp');
  const schemaPath = config.split('\n')[0].split('$schema=')[1];
  assert.equal(JSON.parse(await readFile(resolve(root, schemaPath), 'utf8')).type, 'object');

  const { createHtmlReporter }: typeof import('../src/report/html/index.ts') =
    await import(pathToFileURL(join(installed, 'dist/report/html/index.js')).href);
  const logs: string[] = [];
  const htmlFile = join(root, 'report.html');
  const reporter = await createHtmlReporter({ open: false, htmlFile, log: (line) => logs.push(line) });
  t.after(() => reporter.close());
  const response = await fetch(logs[0].replace('HTML report: ', ''));
  assert.equal(response.status, 200);
  assert.doesNotMatch(await response.text(), /<!--REPORT_(STYLE|DATA|SCRIPT)-->/);
  reporter.onEvent({ type: 'run-started', at: new Date().toISOString(), tests: [] });
  await reporter.close();
  const report = await readFile(htmlFile, 'utf8');
  assert.match(report, /run-started/);
  assert.doesNotMatch(report, /<!--REPORT_(STYLE|DATA|SCRIPT)-->/);
});
