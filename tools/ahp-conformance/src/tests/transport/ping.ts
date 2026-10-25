import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'transport/ping/before-initialize',
    area: 'Transport & lifecycle',
    feature: 'ping',
    title: 'Ping responds with null before initialization.',
    spec: '/reference/common#ping',
    async run(t) {
      const connection = await t.connect();
      assert.equal(await connection.request('ping', { channel: ROOT }), null);
    },
  },
  {
    id: 'transport/ping/after-initialize',
    area: 'Transport & lifecycle',
    feature: 'ping',
    title: 'Ping responds with null after initialization.',
    spec: '/reference/common#ping',
    async run(t) {
      const client = await t.client();
      assert.equal(await client.request('ping', { channel: ROOT }), null);
    },
  },
  {
    id: 'transport/ping/without-subscriptions',
    area: 'Transport & lifecycle',
    feature: 'ping',
    title: 'Ping responds with null when the client holds no subscriptions.',
    spec: '/specification/transport#keep-alive',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      assert.equal(await client.request('ping', { channel: ROOT }), null);
    },
  },
];
