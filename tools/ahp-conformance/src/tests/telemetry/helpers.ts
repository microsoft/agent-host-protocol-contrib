import { assert } from '../../harness/assert.ts';
import type { Client, JsonRpcMessage, Requirement } from '../../harness/types.ts';

export const signals = [
  { name: 'logs', method: 'otlp/exportLogs', resources: 'resourceLogs', scopes: 'scopeLogs', records: 'logRecords' },
  { name: 'traces', method: 'otlp/exportTraces', resources: 'resourceSpans', scopes: 'scopeSpans', records: 'spans' },
  { name: 'metrics', method: 'otlp/exportMetrics', resources: 'resourceMetrics', scopes: 'scopeMetrics', records: 'metrics' },
] as const;
export type Signal = typeof signals[number];
export interface Subscription {
  signal: Signal;
  channel: string;
}

export function record(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

function variables(expression: string): string[] {
  return expression.replace(/^[+#./;?&]/, '').split(',');
}

export function levelTemplate(template: string): boolean {
  return [...template.matchAll(/\{([^{}]+)\}/g)].some((match) =>
    variables(match[1]).some((variable) => /^level(?::[0-9]+|\*)?$/.test(variable)));
}

export const logFiltering: Requirement = {
  label: 'telemetry.logs level template',
  kind: 'capability',
  check: (profile) => typeof profile.initialize.telemetry?.logs === 'string' && levelTemplate(profile.initialize.telemetry.logs),
};

export function expand(template: string, level?: string): string {
  // Only level has a protocol-defined value; unknown variables are omitted.
  const channel = template.replace(/\{([^{}]+)\}/g, (_match, expression: string) => {
    const operator = /^[+#./;?&]/.test(expression) ? expression[0] : '';
    const values = variables(expression).flatMap((variable) => {
      const match = /^level(?::([0-9]+)|\*)?$/.exec(variable);
      if (!match || level === undefined) return [];
      return [encodeURIComponent(match[1] === undefined ? level : level.slice(0, Number(match[1])))];
    });
    if (values.length === 0) return '';
    switch (operator) {
      case '?': case '&': return operator + values.map((value) => `level=${value}`).join('&');
      case ';': return ';' + values.map((value) => `level=${value}`).join(';');
      case '/': case '.': return operator + values.join(operator);
      case '#': return '#' + values.join(',');
      default: return values.join(',');
    }
  });
  assert.ok(!/[{}]/.test(channel), 'telemetry URI template could not be expanded');
  assert.equal(new URL(channel).protocol, 'ahp-otlp:', 'advertised telemetry channels must use ahp-otlp:');
  return channel;
}

export function advertised(client: Client): Subscription[] {
  return signals.flatMap((signal) => {
    const template = client.initializeResult.telemetry?.[signal.name];
    if (template === undefined) return [];
    assert.equal(typeof template, 'string');
    return [{ signal, channel: expand(template) }];
  });
}

export function subscribe(client: Client, subscription: Subscription): Promise<unknown> {
  return client.request('subscribe', { channel: subscription.channel });
}

export async function observe(client: Client, from: number, subscriptions: Subscription[]): Promise<JsonRpcMessage[]> {
  await client.expectNoNotification(subscriptions[0].signal.method, () => false, 400);
  return client.received.slice(from).filter((message) => signals.some((signal) => signal.method === message.method));
}

export function assertExport(message: JsonRpcMessage, subscriptions: Subscription[]): Record<string, unknown>[] {
  assert.equal(message.id, undefined, 'OTLP exports must be notifications');
  assert.equal(message.jsonrpc, '2.0');
  const subscription = subscriptions.find((entry) => entry.signal.method === message.method);
  assert.ok(subscription, `received an export for an unsubscribed signal: ${message.method}`);
  record(message.params);
  assert.equal(message.params.channel, subscription.channel, 'export channel must match the subscribed expanded URI');
  record(message.params.payload);
  const payload = message.params.payload;
  for (const signal of signals) {
    if (signal.name !== subscription.signal.name) {
      assert.equal(payload[signal.resources], undefined, 'payload uses a different OTLP signal request shape');
    }
  }
  const resources = payload[subscription.signal.resources];
  if (resources === undefined) return [];
  assert.ok(Array.isArray(resources));
  const records: Record<string, unknown>[] = [];
  for (const resource of resources) {
    record(resource);
    if (resource.resource !== undefined) record(resource.resource);
    const scopes: unknown = resource[subscription.signal.scopes];
    if (scopes === undefined) continue;
    assert.ok(Array.isArray(scopes));
    for (const scope of scopes) {
      record(scope);
      if (scope.scope !== undefined) record(scope.scope);
      const entries: unknown = scope[subscription.signal.records];
      if (entries === undefined) continue;
      assert.ok(Array.isArray(entries));
      for (const entry of entries) {
        record(entry);
        records.push(entry);
      }
    }
  }
  return records;
}
