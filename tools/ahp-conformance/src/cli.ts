#!/usr/bin/env node
import { access, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { createInterface } from 'node:readline/promises';
import { parseArgs } from 'node:util';
import { parse } from 'yaml';
import { configSchema } from './config/schema.ts';
import { loadConfig } from './config/load.ts';
import { writeTemplate } from './config/template.ts';
import { configuredSecrets, redact } from './harness/redact.ts';
import { AREAS } from './harness/types.ts';
import type { Reporter } from './harness/types.ts';
import { closeReporters, createReporters } from './report/index.ts';
import { run } from './runner/index.ts';
import { selectTests } from './runner/filter.ts';
import { allTests } from './tests/index.ts';

const usage = `Usage: ahp-conformance [config] [options]

  --filter <glob>       Select matching test ids (repeatable)
  --reporter cli|html   Override the reporter
  --no-open            Do not open the HTML report in a browser
  --json <file>        Also write JSON results (path relative to cwd)
  --proposed           Also run tests of proposed, not-yet-ratified protocol behavior
  --init               Create a commented config without overwriting
  --list               List tests without connecting
  --help               Show this help

Default config: ./ahp-conformance.yml`;

async function exists(path: string): Promise<boolean> {
  try { await access(path); return true; }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error; }
}

async function missingConfig(path: string): Promise<boolean> {
  if (!process.stdin.isTTY || !process.stdout.isTTY) throw new Error(`Configuration not found: ${path}. Run ahp-conformance --init to create it.`);
  const prompt = createInterface({ input: process.stdin, output: process.stdout });
  try {
    if (/^n/i.test((await prompt.question(`Create ${path}? [Y/n] `)).trim())) return false;
    const url = (await prompt.question('Endpoint URL [ws://127.0.0.1:8787/ahp]: ')).trim() || 'ws://127.0.0.1:8787/ahp';
    if (!/^wss?:\/\//.test(url)) throw new Error('Endpoint must be a ws:// or wss:// URL');
    new URL(url);
    await writeTemplate(path, url);
    console.log(`Created ${path}`);
    return !/^n/i.test((await prompt.question('Run now? [Y/n] ')).trim());
  } finally { prompt.close(); }
}

async function main(): Promise<number> {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      filter: { type: 'string', multiple: true }, reporter: { type: 'string' }, 'no-open': { type: 'boolean' },
      json: { type: 'string' }, proposed: { type: 'boolean' }, init: { type: 'boolean' }, list: { type: 'boolean' }, help: { type: 'boolean', short: 'h' },
    },
  });
  if (values.help) { console.log(usage); return 0; }
  if (positionals.length > 1) throw new Error('Expected at most one configuration path');
  if (values.reporter && values.reporter !== 'cli' && values.reporter !== 'html') throw new Error('--reporter must be cli or html');
  const path = resolve(positionals[0] ?? 'ahp-conformance.yml');
  if (values.init) {
    await writeTemplate(path);
    console.log(`Created ${path}. Set target.url, then run node src/cli.ts ${JSON.stringify(path)}.`);
    return 0;
  }
  if (values.list) {
    const configured = await exists(path) ? configSchema.parse(parse(await readFile(path, 'utf8'))) : undefined;
    const tests = selectTests(allTests, configured?.run.include, configured?.run.exclude, values.filter);
    for (const area of AREAS) {
      const group = tests.filter((test) => test.area === area);
      if (!group.length) continue;
      console.log(`${area}\n${group.map((test) => `  ${test.id}  ${test.title}`).join('\n')}\n`);
    }
    return 0;
  }
  if (!await exists(path) && !await missingConfig(path)) return 0;
  const config = await loadConfig(path, {
    reporter: values.reporter as 'cli' | 'html' | undefined,
    open: values['no-open'] ? false : undefined, jsonFile: values.json, proposed: values.proposed,
  });
  const controller = new AbortController();
  const signalHandlers = (['SIGINT', 'SIGTERM', 'SIGHUP'] as const).map((signal) => {
    const handler = () => controller.abort(new Error(`Interrupted by ${signal}`));
    process.on(signal, handler);
    return { signal, handler };
  });
  let reporters: Reporter[] = [];
  let exitCode = 2;
  try {
    reporters = await createReporters(config);
    const result = await run(config, reporters, { signal: controller.signal, filters: values.filter });
    exitCode = result.failed || controller.signal.aborted ? 1 : 0;
  } catch (error) {
    console.error(redact(error instanceof Error ? error.message : String(error), configuredSecrets(config)));
  } finally {
    try { await closeReporters(reporters); }
    catch (error) {
      console.error(`Reporter error: ${redact(error instanceof Error ? error.message : String(error), configuredSecrets(config))}`);
      exitCode = 2;
    }
    for (const { signal, handler } of signalHandlers) process.off(signal, handler);
  }
  return exitCode;
}

try { process.exitCode = await main(); }
catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exitCode = 2;
}
