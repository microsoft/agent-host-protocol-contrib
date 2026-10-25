import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { expectRpcFailure, fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

export const tests: TestDefinition[] = [
  resourceTest('resourceDelete', 'file', 'resourceDelete removes an existing file.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'contents');
    await client.request('resourceDelete', { channel: rootChannel, uri: file.uri });
    await assert.rejects(stat(file.path), { code: 'ENOENT' });
  }),
  resourceTest('resourceDelete', 'empty-directory', 'resourceDelete removes an empty directory without recursive mode.', async (t) => {
    const client = await resourceClient(t, true);
    const directory = scratchFile(t, 'directory', '');
    await mkdir(directory.path);
    await client.request('resourceDelete', { channel: rootChannel, uri: directory.uri });
    await assert.rejects(stat(directory.path), { code: 'ENOENT' });
  }),
  resourceTest('resourceDelete', 'nonempty-default', 'resourceDelete rejects deleting a non-empty directory when recursive is omitted.', async (t) => {
    const client = await resourceClient(t, true);
    const directory = scratchFile(t, 'directory', '');
    const child = join(directory.path, `${t.uniqueId('child')}.txt`);
    await mkdir(directory.path);
    await writeFile(child, 'preserved');
    await expectRpcFailure(client.request('resourceDelete', { channel: rootChannel, uri: directory.uri }));
    assert.equal(await readFile(child, 'utf8'), 'preserved');
  }),
  resourceTest('resourceDelete', 'recursive-directory', 'resourceDelete with recursive enabled removes a directory and its descendants.', async (t) => {
    const client = await resourceClient(t, true);
    const directory = scratchFile(t, 'directory', '');
    const nested = join(directory.path, t.uniqueId('nested'));
    await mkdir(nested, { recursive: true });
    await writeFile(join(nested, `${t.uniqueId('child')}.txt`), 'contents');
    await client.request('resourceDelete', { channel: rootChannel, uri: directory.uri, recursive: true });
    await assert.rejects(stat(directory.path), { code: 'ENOENT' });
  }),
  resourceTest('resourceDelete', 'missing-resource', 'resourceDelete returns NotFound for a missing resource.', async (t) => {
    const client = await resourceClient(t, true);
    await expectRpcError(client.request('resourceDelete', { channel: rootChannel, uri: scratchFile(t, 'missing').uri }), AhpErrorCodes.NotFound);
  }),
];
