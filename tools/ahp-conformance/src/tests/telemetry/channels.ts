import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { advertised, assertExport, expand, levelTemplate, logFiltering, observe, record, signals, subscribe } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'telemetry/subscribe/stateless-result',
    area: 'Telemetry',
    feature: 'subscribe',
    title: 'Advertised telemetry channels return empty stateless subscription results.',
    spec: '/specification/telemetry-channel#subscribing',
    requires: [requires.telemetry],
    async run(t) {
      const client = await t.client();
      const subscriptions = advertised(client);
      if (subscriptions.length === 0) t.skip('This connection does not advertise an OTLP signal URI.');
      for (const subscription of subscriptions) {
        assert.deepEqual(await subscribe(client, subscription), {}, 'telemetry subscribe must return an empty stateless result');
      }
    },
  },
  {
    id: 'telemetry/export/signal-envelope',
    area: 'Telemetry',
    feature: 'otlp/export',
    title: 'Observed OTLP exports use the subscribed signal channel and request payload shape.',
    spec: '/specification/telemetry-channel#wire-format',
    requires: [requires.telemetry],
    async run(t) {
      const client = await t.client();
      const subscriptions = advertised(client);
      if (subscriptions.length === 0) t.skip('This connection does not advertise an OTLP signal URI.');
      const from = client.received.length;
      for (const subscription of subscriptions) await subscribe(client, subscription);
      const messages = await observe(client, from, subscriptions);
      if (messages.length === 0) t.skip('No OTLP batch arrived during the 400 ms observation window.');
      for (const message of messages) assertExport(message, subscriptions);
    },
  },
  {
    id: 'telemetry/logs/level-case-insensitive-subscription',
    area: 'Telemetry',
    feature: 'logs filtering',
    title: 'SHOULD: Mixed-case log-level subscriptions preserve their exact channel URI.',
    spec: '/specification/telemetry-channel#filtering',
    requires: [requires.telemetry, logFiltering],
    async run(t) {
      const client = await t.client();
      const template = client.initializeResult.telemetry?.logs;
      if (template === undefined || !levelTemplate(template)) {
        return t.skip('This connection no longer advertises a logs level template.');
      }
      const subscription = { signal: signals[0], channel: expand(template, 'WaRn') };
      const from = client.received.length;
      await subscribe(client, subscription);
      const messages = (await observe(client, from, [subscription])).filter((message) =>
        message.method === 'otlp/exportLogs' && message.id === undefined);
      if (messages.length === 0) return t.skip('No log export arrived during the 400 ms observation window.');
      for (const message of messages) {
        record(message.params);
        assert.equal(message.params.channel, subscription.channel, 'mixed-case subscription must retain its exact channel URI');
      }
    },
  },
  {
    id: 'telemetry/logs/severity-filtering',
    area: 'Telemetry',
    feature: 'logs filtering',
    title: 'SHOULD: Log subscriptions independently honor their advertised minimum severity.',
    spec: '/specification/telemetry-channel#filtering',
    requires: [requires.telemetry, logFiltering],
    async run(t) {
      const observations = await Promise.all([
        { level: 'info', minimum: 9 }, { level: 'warn', minimum: 13 },
      ].map(async ({ level, minimum }) => {
        const client = await t.client();
        const template = client.initializeResult.telemetry?.logs;
        if (template === undefined) return t.skip('This connection no longer advertises a logs URI.');
        const subscription = { signal: signals[0], channel: expand(template, level) };
        const from = client.received.length;
        await subscribe(client, subscription);
        return { client, subscription, from, minimum };
      }));
      const batches = await Promise.all(observations.map(({ client, from, subscription }) => observe(client, from, [subscription])));
      let streamsWithRecords = 0;
      for (const [index, messages] of batches.entries()) {
        let count = 0;
        for (const message of messages) {
          for (const entry of assertExport(message, [observations[index].subscription])) {
            const severity = entry.severityNumber ?? 0;
            assert.equal(typeof severity, 'number');
            assert.ok(Number.isInteger(severity));
            assert.ok((severity as number) >= observations[index].minimum, 'filtered log severity was below the requested band');
            count++;
          }
        }
        if (count > 0) streamsWithRecords++;
      }
      if (streamsWithRecords < observations.length) {
        t.skip('Both filtered streams did not produce log records during the 400 ms observation window.');
      }
    },
  },
];
