import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { ResourceListResult } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { expectRpcFailure, fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

export const tests: TestDefinition[] = [
  resourceTest('resourceList', 'entry-types', 'resourceList reports directly contained files and directories with their correct types.', async (t) => {
    const client = await resourceClient(t);
    const directory = scratchFile(t, 'directory', '');
    const file = scratchFile(t);
    const child = t.uniqueId('child');
    await mkdir(directory.path);
    await writeFile(file.path, 'contents');
    await writeFile(join(directory.path, child), 'nested');
    const result = await client.request<ResourceListResult>('resourceList', { channel: rootChannel, uri: t.scratchUri() });
    assert.equal(result.entries.find((entry) => entry.name === directory.name)?.type, 'directory');
    assert.equal(result.entries.find((entry) => entry.name === file.name)?.type, 'file');
    assert.ok(!result.entries.some((entry) => entry.name === child), 'nested entries must not be returned as direct children');
  }),
  resourceTest('resourceList', 'missing-directory', 'resourceList returns NotFound for a missing directory.', async (t) => {
    const client = await resourceClient(t);
    await expectRpcError(client.request('resourceList', { channel: rootChannel, uri: scratchFile(t, 'missing', '').uri }), AhpErrorCodes.NotFound);
  }),
  resourceTest('resourceList', 'not-directory', 'resourceList returns a JSON-RPC error when the target is a file.', async (t) => {
    const client = await resourceClient(t);
    const file = await fixtureFile(t, 'contents');
    await expectRpcFailure(client.request('resourceList', { channel: rootChannel, uri: file.uri }));
  }),
];
