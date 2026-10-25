import { JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { Connection, JsonRpcMessage, TestDefinition } from '../../harness/types.ts';
import { expectNoResponses, initialize, ROOT } from './helpers.ts';

function errorObject(message: JsonRpcMessage): { code: number; message: string } {
  assert.ok(message.error !== null && typeof message.error === 'object');
  return message.error as { code: number; message: string };
}

// AHP 1.0.0 does not mandate command-parameter validation; JSON-RPC 4.2 permits
// absent parameter names without an error. Check framing, not a particular policy/code.
async function expectParameterResponse(connection: Connection, request: () => Promise<unknown>): Promise<void> {
  const start = connection.received.length;
  try { await request(); }
  catch (error) { assert.ok(error instanceof RpcError, 'expected a JSON-RPC response, not a transport failure'); }
  const responses = connection.received.slice(start).filter((message) =>
    Object.hasOwn(message, 'result') || Object.hasOwn(message, 'error'));
  assert.equal(responses.length, 1);
  const response = responses[0]!;
  assert.equal(response.jsonrpc, '2.0');
  assert.notEqual(Object.hasOwn(response, 'result'), Object.hasOwn(response, 'error'));
  if (Object.hasOwn(response, 'error')) {
    const error = errorObject(response);
    assert.ok(Number.isInteger(error.code));
    assert.equal(typeof error.message, 'string');
  }
}

const invalidRequests = [
  { name: 'missing-method', message: { jsonrpc: '2.0', id: 71 } },
  { name: 'non-string-method', message: { jsonrpc: '2.0', id: 72, method: 42, params: { channel: ROOT } } },
  { name: 'missing-jsonrpc', message: { id: 73, method: 'ping', params: { channel: ROOT } } },
  { name: 'wrong-jsonrpc-version', message: { jsonrpc: '1.0', id: 74, method: 'ping', params: { channel: ROOT } } },
];

export const tests: TestDefinition[] = [
  {
    id: 'transport/json-rpc/exactly-one-response',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Every request receives exactly one response with its request id.',
    spec: '/specification/overview#requests',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const start = client.received.length;
      const ids = [910001, 910002, 910003, 910004];
      for (const [index, id] of ids.entries()) {
        client.sendRaw(JSON.stringify({
          jsonrpc: '2.0', id,
          method: index === ids.length - 1 ? t.uniqueId('unknown-method') : 'ping',
          params: { channel: ROOT },
        }));
      }
      await Promise.all(ids.map((id) => client.waitForMessage((message) =>
        message.id === id && (Object.hasOwn(message, 'result') || Object.hasOwn(message, 'error')))));
      await client.expectNoNotification('action', () => false, 350);
      const responses = client.received.slice(start).filter((message) =>
        Object.hasOwn(message, 'result') || Object.hasOwn(message, 'error'));
      assert.equal(responses.length, ids.length, 'requests must not produce extra or uncorrelated responses');
      for (const id of ids) assert.equal(responses.filter((response) => response.id === id).length, 1,
        `request ${id} must receive exactly one response`);
    },
  },
  {
    id: 'transport/json-rpc/success-envelope',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A successful response has the JSON-RPC version, matching id, result, and no error.',
    spec: '/reference/common#jsonrpcsuccessresponse',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const id = 910005;
      client.sendRaw(JSON.stringify({ jsonrpc: '2.0', id, method: 'ping', params: { channel: ROOT } }));
      const response = await client.waitForMessage((message) => message.id === id);
      assert.equal(response.jsonrpc, '2.0');
      assert.equal(response.id, id);
      assert.ok(Object.hasOwn(response, 'result'), 'a success response must carry result even when null');
      assert.equal(Object.hasOwn(response, 'error'), false);
      assert.equal(response.result, null);
    },
  },
  {
    id: 'transport/json-rpc/malformed-json',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Malformed JSON receives ParseError with a null response id.',
    spec: '/reference/error-codes#standard-json-rpc-codes',
    async run(t) {
      const client = await t.client();
      client.sendRaw('{"jsonrpc":"2.0","id":');
      const response = await client.waitForMessage((message) => Object.hasOwn(message, 'error'));
      assert.equal(errorObject(response).code, JsonRpcErrorCodes.ParseError);
      assert.equal(response.id, null);
    },
  },
  ...invalidRequests.map(({ name, message }): TestDefinition => ({
    id: `transport/json-rpc/${name}`,
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: `A request with ${name.replaceAll('-', ' ')} receives InvalidRequest.`,
    spec: '/reference/error-codes#standard-json-rpc-codes',
    async run(t) {
      const client = await t.client();
      client.sendRaw(JSON.stringify(message));
      const response = await client.waitForMessage((received) =>
        (received.id === message.id || received.id === null) &&
        (Object.hasOwn(received, 'result') || Object.hasOwn(received, 'error')));
      assert.equal(errorObject(response).code, JsonRpcErrorCodes.InvalidRequest);
    },
  })),
  {
    id: 'transport/json-rpc/non-object-request',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A JSON scalar receives InvalidRequest with a null response id.',
    spec: '/reference/error-codes#standard-json-rpc-codes',
    async run(t) {
      const client = await t.client();
      client.sendRaw('42');
      const response = await client.waitForMessage((message) => Object.hasOwn(message, 'error'));
      assert.equal(errorObject(response).code, JsonRpcErrorCodes.InvalidRequest);
      assert.equal(response.id, null);
    },
  },
  {
    id: 'transport/json-rpc/unknown-method',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'An unknown request method receives MethodNotFound.',
    spec: '/reference/error-codes#standard-json-rpc-codes',
    async run(t) {
      const client = await t.client();
      await expectRpcError(client.request(t.uniqueId('unknown-method'), { channel: ROOT }), JsonRpcErrorCodes.MethodNotFound);
    },
  },
  {
    id: 'transport/json-rpc/missing-required-parameter',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A subscribe request without channel receives a well-formed JSON-RPC response.',
    spec: '/specification/overview#requests',
    async run(t) {
      const client = await t.client();
      await expectParameterResponse(client, () => client.request('subscribe', {}));
    },
  },
  {
    id: 'transport/json-rpc/wrong-parameter-type',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Initialization with non-array protocolVersions receives a well-formed JSON-RPC response.',
    spec: '/specification/overview#requests',
    async run(t) {
      const connection = await t.connect();
      await expectParameterResponse(connection, () => initialize(t, connection, { protocolVersions: '1.0.0' }));
    },
  },
  {
    id: 'transport/json-rpc/missing-client-id',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Initialization without clientId receives a well-formed JSON-RPC response.',
    spec: '/specification/overview#requests',
    async run(t) {
      const connection = await t.connect();
      await expectParameterResponse(connection, () => initialize(t, connection, { clientId: undefined }));
    },
  },
  {
    id: 'transport/json-rpc/request-id-echo',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A request response echoes the numeric request id.',
    spec: '/reference/common#jsonrpcsuccessresponse',
    async run(t) {
      const client = await t.client();
      const id = 9007199254740990;
      client.sendRaw(JSON.stringify({ jsonrpc: '2.0', id, method: 'ping', params: { channel: ROOT } }));
      const response = await client.waitForMessage((message) => message.id === id);
      assert.ok(Object.hasOwn(response, 'result'));
      assert.equal(response.result, null);
    },
  },
  {
    id: 'transport/json-rpc/error-response-shape',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'An error response carries a numeric code and string message instead of a result.',
    spec: '/reference/error-codes#error-response-format',
    async run(t) {
      const client = await t.client();
      const id = 9007199254740989;
      client.sendRaw(JSON.stringify({ jsonrpc: '2.0', id, method: t.uniqueId('unknown-method'), params: { channel: ROOT } }));
      const response = await client.waitForMessage((message) => message.id === id);
      const error = errorObject(response);
      assert.equal(typeof error.code, 'number');
      assert.equal(typeof error.message, 'string');
      assert.equal(Object.hasOwn(response, 'result'), false);
    },
  },
  {
    id: 'transport/json-rpc/unknown-request-field',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Known commands ignore unknown optional parameter fields.',
    spec: '/specification/versioning#compatibility-guarantee',
    async run(t) {
      const client = await t.client();
      assert.equal(await client.request('ping', { channel: ROOT, [t.uniqueId('future-field')]: ['ignored'] }), null);
    },
  },
  {
    id: 'transport/json-rpc/unknown-request-metadata',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'Known commands ignore unknown metadata keys.',
    spec: '/reference/common#baseparams',
    async run(t) {
      const client = await t.client();
      assert.equal(await client.request('ping', {
        channel: ROOT,
        _meta: { [`conformance.example/${t.uniqueId('extension')}`]: { nested: true } },
      }), null);
    },
  },
  {
    id: 'transport/json-rpc/known-method-notification-no-response',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A known method sent without an id receives no response.',
    spec: '/reference/common#jsonrpcnotification',
    async run(t) {
      const client = await t.client();
      await expectNoResponses(client, () => client.notify('ping', { channel: ROOT }));
    },
  },
  {
    id: 'transport/json-rpc/unknown-notification-no-response',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'An unknown notification method receives no response.',
    spec: '/reference/common#jsonrpcnotification',
    async run(t) {
      const client = await t.client();
      await expectNoResponses(client, () => client.notify(t.uniqueId('unknown-notification'), { channel: ROOT }));
    },
  },
  {
    id: 'transport/json-rpc/invalid-params-notification-no-response',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'A notification with invalid method parameters receives no response.',
    spec: '/reference/common#jsonrpcnotification',
    async run(t) {
      const client = await t.client();
      await expectNoResponses(client, () => client.notify('subscribe', { channel: 42 }));
    },
  },
  {
    id: 'transport/json-rpc/unsubscribe-no-response',
    area: 'Transport & lifecycle',
    feature: 'JSON-RPC',
    title: 'An unsubscribe notification receives no response.',
    spec: '/specification/subscriptions#unsubscribe-notification',
    async run(t) {
      const client = await t.client();
      await expectNoResponses(client, () => client.notify('unsubscribe', { channel: ROOT }));
    },
  },
];
