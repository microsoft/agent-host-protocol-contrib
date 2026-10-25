import { AssertionError } from 'node:assert';
import { randomUUID } from 'node:crypto';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { Context, TestSkipped } from '../harness/context.ts';
import { configuredSecrets, redact } from '../harness/redact.ts';
import type { ResolvedConfig } from '../config/schema.ts';
import { removeScratchDir } from '../config/workspace.ts';
import type { Reporter, RunEvent, TestDefinition, TestSummary } from '../harness/types.ts';
import { launchHost } from '../host/index.ts';
import { allTests } from '../tests/index.ts';
import { selectTests } from './filter.ts';
import { probe, summarizeProfile } from './profile.ts';
import { skipReason } from './requirements.ts';

export function summarizeTest(test: TestDefinition): TestSummary {
  return { id: test.id, area: test.area, feature: test.feature, title: test.title, spec: test.spec, requires: test.requires?.map((requirement) => requirement.label) ?? [] };
}

export async function run(config: ResolvedConfig, reporters: readonly Reporter[], options: { signal: AbortSignal; filters?: string[]; tests?: TestDefinition[] }): Promise<{ passed: number; failed: number; skipped: number }> {
  const start = Date.now();
  const secrets = configuredSecrets(config);
  const emit = (event: RunEvent) => {
    let safe = event;
    if (event.type === 'test-frame') safe = { ...event, frame: { ...event.frame, message: redact(event.frame.message, secrets) } };
    else if (event.type === 'test-log') safe = { ...event, message: redact(event.message, secrets) as string, data: redact(event.data, secrets) };
    else if (event.type === 'run-error') safe = { ...event, message: redact(event.message, secrets) as string };
    else if (event.type === 'test-finished') safe = { ...event, reason: redact(event.reason, secrets) as string | undefined, error: redact(event.error, secrets) as typeof event.error };
    else if (event.type === 'profile') safe = { ...event, profile: redact(event.profile, secrets) as typeof event.profile };
    for (const reporter of reporters) reporter.onEvent(safe);
  };
  const counts = { passed: 0, failed: 0, skipped: 0 };
  let host: Awaited<ReturnType<typeof launchHost>>;
  try {
    const tests = selectTests(options.tests ?? allTests, config.run.include, config.run.exclude, options.filters);
    host = await launchHost(config, options.signal, emit);
    const profile = await probe(config, options.signal, emit);
    emit({ type: 'profile', at: new Date().toISOString(), profile: summarizeProfile(profile) });
    emit({ type: 'run-started', at: new Date().toISOString(), tests: tests.map(summarizeTest) });
    const runnable: TestDefinition[] = [];
    for (const test of tests) {
      const reason = skipReason(test, profile);
      if (reason) {
        counts.skipped++;
        emit({ type: 'test-finished', at: new Date().toISOString(), id: test.id, status: 'skipped', durationMs: 0, reason });
      } else runnable.push(test);
    }
    const execute = async (test: TestDefinition) => {
      const started = Date.now();
      const controller = new AbortController();
      const cancelled = () => controller.abort(options.signal.reason ?? new Error('Run cancelled'));
      options.signal.addEventListener('abort', cancelled, { once: true });
      if (options.signal.aborted) cancelled();
      const timeoutMs = test.timeoutMs ?? config.run.timeoutMs;
      const timer = setTimeout(() => controller.abort(new Error(`timed out after ${timeoutMs} ms`)), timeoutMs);
      const name = `${test.id.replace(/[^a-zA-Z0-9._-]/g, '-').slice(0, 100)}-${randomUUID().slice(0, 8)}`;
      const scratchDir = join(config.workspace.scratchDir, name);
      const context = new Context({ id: test.id, profile, signal: controller.signal, scratchDir, timeoutMs, emit });
      let status: 'passed' | 'failed' | 'skipped' = 'passed';
      let reason: string | undefined;
      let detail: Extract<RunEvent, { type: 'test-finished' }>['error'];
      emit({ type: 'test-started', at: new Date().toISOString(), id: test.id });
      try {
        controller.signal.throwIfAborted();
        await mkdir(scratchDir, { recursive: true });
        await new Promise<void>((resolve, reject) => {
          const aborted = () => reject(controller.signal.reason);
          controller.signal.addEventListener('abort', aborted, { once: true });
          Promise.resolve().then(() => test.run(context)).then(resolve, reject).finally(() => controller.signal.removeEventListener('abort', aborted));
          if (controller.signal.aborted) aborted();
        });
      } catch (error) {
        status = error instanceof TestSkipped ? 'skipped' : 'failed';
        reason = error instanceof Error ? error.message : String(error);
        if (status === 'failed') detail = {
          message: reason, ...(error instanceof Error && { stack: error.stack }),
          ...(error instanceof AssertionError && { expected: error.expected, actual: error.actual }),
        };
      } finally {
        clearTimeout(timer);
        options.signal.removeEventListener('abort', cancelled);
        for (const error of await context.cleanup()) context.log('Cleanup failed', error instanceof Error ? error.message : String(error));
      }
      counts[status]++;
      emit({ type: 'test-finished', at: new Date().toISOString(), id: test.id, status, durationMs: Date.now() - started, reason, error: detail });
    };
    let next = 0;
    await Promise.all(Array.from({ length: Math.min(config.run.concurrency, runnable.length) }, async () => {
      while (next < runnable.length) {
        const test = runnable[next++];
        if (options.signal.aborted) {
          counts.skipped++;
          emit({ type: 'test-finished', at: new Date().toISOString(), id: test.id, status: 'skipped', durationMs: 0, reason: 'Run cancelled' });
        } else await execute(test);
      }
    }));
    emit({ type: 'run-finished', at: new Date().toISOString(), ...counts, durationMs: Date.now() - start });
    return counts;
  } catch (error) {
    emit({ type: 'run-error', at: new Date().toISOString(), message: error instanceof Error ? error.message : String(error) });
    throw error;
  } finally {
    try { await host?.stop(); }
    finally {
      if (!config.workspace.keep) await removeScratchDir(config.workspace.scratchDir);
    }
  }
}
