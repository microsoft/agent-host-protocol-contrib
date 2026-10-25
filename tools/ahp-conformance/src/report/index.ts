import type { ResolvedConfig } from '../config/schema.ts';
import type { Reporter } from '../harness/types.ts';
import { createCliReporter } from './cli.ts';
import { createHtmlReporter } from './html/index.ts';
import { createJsonReporter } from './json.ts';

export async function createReporters(config: ResolvedConfig): Promise<Reporter[]> {
  const reporters: Reporter[] = [];
  if (config.report.reporter === 'html') {
    reporters.push(await createHtmlReporter({ open: config.report.open, htmlFile: config.report.htmlFile, log: console.log }));
    reporters.push(createCliReporter({ summaryOnly: true }));
  } else reporters.push(createCliReporter());
  if (config.report.jsonFile) reporters.push(createJsonReporter(config.report.jsonFile));
  return reporters;
}

export async function closeReporters(reporters: readonly Reporter[]): Promise<void> {
  const results = await Promise.allSettled(reporters.map((reporter) => reporter.close()));
  const failures = results.filter((result): result is PromiseRejectedResult => result.status === 'rejected');
  if (failures.length) throw new AggregateError(failures.map((result) => result.reason), 'Could not finish reports');
}
