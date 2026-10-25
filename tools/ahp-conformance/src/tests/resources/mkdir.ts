import { mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

const spec = '/reference/common#resourcemkdir';

export const tests: TestDefinition[] = [
  resourceTest('resourceMkdir', 'creates-directory', 'resourceMkdir creates a directory at the requested URI.', async (t) => {
    const client = await resourceClient(t, true);
    const directory = scratchFile(t, 'directory', '');
    await client.request('resourceMkdir', { channel: rootChannel, uri: directory.uri });
    assert.ok((await stat(directory.path)).isDirectory());
  }, spec),
  resourceTest('resourceMkdir', 'creates-parents', 'resourceMkdir creates missing parent directories.', async (t) => {
    const client = await resourceClient(t, true);
    const relative = `${t.uniqueId('parent')}/${t.uniqueId('child')}/${t.uniqueId('leaf')}`;
    await client.request('resourceMkdir', { channel: rootChannel, uri: t.scratchUri(relative) });
    assert.ok((await stat(join(t.scratchDir, relative))).isDirectory());
  }, spec),
  resourceTest('resourceMkdir', 'existing-directory', 'resourceMkdir succeeds without changing an existing directory’s contents.', async (t) => {
    const client = await resourceClient(t, true);
    const directory = scratchFile(t, 'directory', '');
    const child = join(directory.path, `${t.uniqueId('child')}.txt`);
    await mkdir(directory.path);
    await writeFile(child, 'preserved');
    await client.request('resourceMkdir', { channel: rootChannel, uri: directory.uri });
    assert.equal(await readFile(child, 'utf8'), 'preserved');
  }, spec),
  resourceTest('resourceMkdir', 'existing-file', 'resourceMkdir returns AlreadyExists when the requested URI is a file.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'preserved');
    await expectRpcError(client.request('resourceMkdir', { channel: rootChannel, uri: file.uri }), AhpErrorCodes.AlreadyExists);
    assert.equal(await readFile(file.path, 'utf8'), 'preserved');
  }, spec),
];
