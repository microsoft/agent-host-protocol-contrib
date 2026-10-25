import { writeFile } from 'node:fs/promises';
import type { FrameRecord, ProfileSummary, Reporter, RunEvent, TestStatus, TestSummary } from '../harness/types.ts';

interface JsonTest extends TestSummary {
  status: TestStatus;
  durationMs: number;
  reason?: string;
  error?: Extract<RunEvent, { type: 'test-finished' }>['error'];
  logs: { at: string; message: string; data?: unknown }[];
  /** Redacted wire transcript, retained only for failed tests. */
  frames?: (FrameRecord & { at: string })[];
}

export function createJsonReporter(file: string): Reporter {
  const tests = new Map<string, JsonTest>();
  let profile: ProfileSummary | undefined;
  let summary: Extract<RunEvent, { type: 'run-finished' }> | undefined;
  const errors: string[] = [];
  return {
    onEvent(event) {
      if (event.type === 'profile') profile = event.profile;
      else if (event.type === 'run-started') {
        for (const test of event.tests) tests.set(test.id, { ...test, status: 'queued', durationMs: 0, logs: [] });
      } else if (event.type === 'run-error') errors.push(event.message);
      else if (event.type === 'run-finished') summary = event;
      else if ('id' in event) {
        const test = tests.get(event.id);
        if (!test) return;
        if (event.type === 'test-started') test.status = 'running';
        else if (event.type === 'test-frame') (test.frames ??= []).push({ at: event.at, ...event.frame });
        else if (event.type === 'test-log') test.logs.push({ at: event.at, message: event.message, ...(event.data !== undefined && { data: event.data }) });
        else if (event.type === 'test-finished') {
          Object.assign(test, { status: event.status, durationMs: event.durationMs, reason: event.reason, error: event.error });
          if (event.status !== 'failed') delete test.frames;
        }
      }
    },
    async close() {
      await writeFile(file, `${JSON.stringify({ profile, summary, errors, tests: [...tests.values()] }, null, 2)}\n`);
    },
  };
}
