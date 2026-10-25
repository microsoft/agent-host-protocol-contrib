import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { ResourceReadResult } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { decoded, fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

export const tests: TestDefinition[] = [
  resourceTest('resourceRead', 'binary-utf8-fallback', 'resourceRead falls back to base64 without losing binary bytes when UTF-8 is preferred.', async (t) => {
    const client = await resourceClient(t);
    const bytes = Buffer.from([0, 255, 128, 195, 40, 254, 13, 10]);
    const file = await fixtureFile(t, bytes, '.bin');
    const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: file.uri, encoding: 'utf-8' });
    assert.equal(result.encoding, 'base64');
    assert.deepEqual(decoded(result), bytes);
  }, '/reference/common#resourcereadresult'),
  resourceTest('resourceRead', 'utf8-content', 'resourceRead returns the complete contents of a UTF-8 file.', async (t) => {
    const client = await resourceClient(t);
    const text = `${t.uniqueId('text')} — café 🌍\nsecond line\n`;
    const file = await fixtureFile(t, text);
    const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: file.uri });
    assert.deepEqual(decoded(result), Buffer.from(text));
  }),
  resourceTest('resourceRead', 'preferred-utf8', 'SHOULD: resourceRead honors a requested UTF-8 encoding for text.', async (t) => {
    const client = await resourceClient(t);
    const file = await fixtureFile(t, t.uniqueId('text'));
    const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: file.uri, encoding: 'utf-8' });
    assert.equal(result.encoding, 'utf-8');
  }, '/reference/common#resourceread'),
  resourceTest('resourceRead', 'binary-base64', 'resourceRead returns binary bytes exactly in base64 encoding.', async (t) => {
    const client = await resourceClient(t);
    const bytes = Buffer.from([0, 255, 128, 1, 13, 10, 195, 40, 254, 0]);
    const file = await fixtureFile(t, bytes, '.bin');
    const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: file.uri, encoding: 'base64' });
    assert.equal(result.encoding, 'base64');
    assert.deepEqual(decoded(result), bytes);
  }),
  resourceTest('resourceRead', 'missing-file', 'resourceRead returns NotFound for a missing file.', async (t) => {
    const client = await resourceClient(t);
    await expectRpcError(client.request('resourceRead', { channel: rootChannel, uri: scratchFile(t, 'missing').uri }), AhpErrorCodes.NotFound);
  }),
];
