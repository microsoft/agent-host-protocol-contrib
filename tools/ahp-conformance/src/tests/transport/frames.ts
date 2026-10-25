import { once } from 'node:events';
import WebSocket from 'ws';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { ROOT } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'transport/frames/server-text-opcode',
    area: 'Transport & lifecycle', feature: 'frames',
    title: 'The server sends JSON-RPC responses as WebSocket text messages.',
    spec: '/specification/transport#common-transports',
    async run(t) {
      t.signal.throwIfAborted();
      // Connection deliberately exposes parsed JSON only, so inspect the opcode on a local socket.
      const socket = new WebSocket(t.config.target.url, { headers: t.config.target.headers });
      const abort = () => socket.terminate();
      t.signal.addEventListener('abort', abort, { once: true });
      t.defer(() => {
        t.signal.removeEventListener('abort', abort);
        socket.terminate();
      });
      socket.on('error', () => {});
      await once(socket, 'open', { signal: t.signal });
      const response = once(socket, 'message', { signal: t.signal });
      const id = 910006;
      socket.send(JSON.stringify({ jsonrpc: '2.0', id, method: 'ping', params: { channel: ROOT } }));
      const [data, isBinary] = await response;
      t.log('Received WebSocket application message', { isBinary });
      assert.equal(isBinary, false, 'JSON-RPC must use a text message, not binary JSON');
      const message = JSON.parse(data.toString()) as { id?: unknown };
      assert.equal(message.id, id);
    },
  },
  {
    id: 'transport/frames/one-message-per-text-frame',
    area: 'Transport & lifecycle',
    feature: 'frames',
    title: 'A complete JSON-RPC request in a text frame is processed independently.',
    spec: '/specification/transport#common-transports',
    async run(t) {
      const client = await t.client();
      const ids = [9007199254740987, 9007199254740988];
      for (const id of ids) {
        client.sendRaw(JSON.stringify({ jsonrpc: '2.0', id, method: 'ping', params: { channel: ROOT } }));
      }
      for (const id of ids) {
        const response = await client.waitForMessage((message) => message.id === id);
        assert.equal(response.result, null);
      }
    },
  },
];
