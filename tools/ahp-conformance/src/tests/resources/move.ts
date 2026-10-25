import { readFile, stat } from 'node:fs/promises';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

export const tests: TestDefinition[] = [
  resourceTest('resourceMove', 'file', 'resourceMove moves a file to a new destination.', async (t) => {
    const client = await resourceClient(t, true);
    const contents = t.uniqueId('contents');
    const source = await fixtureFile(t, contents);
    const destination = scratchFile(t, 'destination');
    await client.request('resourceMove', { channel: rootChannel, source: source.uri, destination: destination.uri });
    assert.equal(await readFile(destination.path, 'utf8'), contents);
    await assert.rejects(stat(source.path), { code: 'ENOENT' });
  }),
  resourceTest('resourceMove', 'overwrite-default', 'resourceMove overwrites an existing destination by default.', async (t) => {
    const client = await resourceClient(t, true);
    const source = await fixtureFile(t, 'new');
    const destination = await fixtureFile(t, 'a longer old destination');
    await client.request('resourceMove', { channel: rootChannel, source: source.uri, destination: destination.uri });
    assert.equal(await readFile(destination.path, 'utf8'), 'new');
  }),
  resourceTest('resourceMove', 'fail-if-exists', 'resourceMove with failIfExists returns AlreadyExists without changing either file.', async (t) => {
    const client = await resourceClient(t, true);
    const source = await fixtureFile(t, 'source');
    const destination = await fixtureFile(t, 'destination');
    await expectRpcError(client.request('resourceMove', { channel: rootChannel, source: source.uri, destination: destination.uri, failIfExists: true }), AhpErrorCodes.AlreadyExists);
    assert.equal(await readFile(source.path, 'utf8'), 'source');
    assert.equal(await readFile(destination.path, 'utf8'), 'destination');
  }),
  resourceTest('resourceMove', 'missing-source', 'resourceMove returns NotFound for a missing source.', async (t) => {
    const client = await resourceClient(t, true);
    await expectRpcError(client.request('resourceMove', { channel: rootChannel, source: scratchFile(t, 'missing').uri, destination: scratchFile(t, 'destination').uri }), AhpErrorCodes.NotFound);
  }),
];
