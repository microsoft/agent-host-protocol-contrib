import { inspect } from 'node:util';
import type { Reporter, RunEvent, TestSummary } from '../harness/types.ts';

type Finished = Extract<RunEvent, { type: 'test-finished' }>;

export function createCliReporter(options: { summaryOnly?: boolean; log?: (line: string) => void; tty?: boolean } = {}): Reporter {
  const log = options.log ?? console.log;
  const tty = options.tty ?? Boolean(process.stdout.isTTY);
  const tests = new Map<string, TestSummary>();
  const results: Finished[] = [];
  const logs = new Map<string, string[]>();
  const color = (text: string, code: number) => tty ? `\u001b[${code}m${text}\u001b[0m` : text;
  const describe = (event: Finished) => `${event.id}: ${tests.get(event.id)?.title ?? ''}`;
  return {
    onEvent(event) {
      if (event.type === 'run-started') {
        for (const test of event.tests) tests.set(test.id, test);
        if (!options.summaryOnly) log(`Running ${event.tests.length} AHP conformance tests`);
      } else if (event.type === 'profile' && !options.summaryOnly) {
        log(`${event.profile.endpoint} · AHP ${event.profile.protocolVersion} · ${event.profile.agent.displayName}${event.profile.model ? ` / ${event.profile.model}` : ''}`);
      } else if (event.type === 'test-log') {
        const entries = logs.get(event.id) ?? [];
        entries.push(`${event.message}${event.data === undefined ? '' : ` ${inspect(event.data, { depth: 5, colors: false })}`}`);
        logs.set(event.id, entries);
      } else if (event.type === 'test-finished') {
        results.push(event);
        if (!options.summaryOnly) {
          const symbol = event.status === 'passed' ? color('✓', 32) : event.status === 'failed' ? color('✗', 31) : color('○', 33);
          log(`${symbol} ${describe(event)} (${Math.round(event.durationMs)} ms)${event.reason ? ` — ${event.reason}` : ''}`);
        }
      } else if (event.type === 'run-error') {
        log(color(`Run error: ${event.message}`, 31));
      } else if (event.type === 'run-finished') {
        if (!options.summaryOnly) {
          log('');
          const areas = new Set([...tests.values()].map((test) => test.area));
          for (const area of areas) {
            const group = results.filter((result) => tests.get(result.id)?.area === area);
            log(`${area}: ${group.filter((result) => result.status === 'passed').length} passed, ${group.filter((result) => result.status === 'failed').length} failed, ${group.filter((result) => result.status === 'skipped').length} skipped`);
          }
          for (const result of results.filter((result) => result.status === 'failed')) {
            log(`\n${color(`FAIL ${describe(result)}`, 31)}`);
            log(result.error?.stack ?? result.reason ?? 'Unknown failure');
            if (result.error && ('expected' in result.error || 'actual' in result.error)) {
              log(`Expected: ${inspect(result.error.expected, { depth: 8, colors: false })}`);
              log(`Actual:   ${inspect(result.error.actual, { depth: 8, colors: false })}`);
            }
            for (const entry of logs.get(result.id) ?? []) log(`  ${entry}`);
          }
        }
        log(`${event.passed} passed, ${event.failed} failed, ${event.skipped} skipped (${(event.durationMs / 1000).toFixed(1)} s)`);
      }
    },
    async close() {},
  };
}
