import { readFile, writeFile } from 'node:fs/promises';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { etag, fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

const spec = '/reference/common#resourcewrite';

export const tests: TestDefinition[] = [
  resourceTest('resourceWrite', 'creates-file', 'resourceWrite creates a missing file with the supplied UTF-8 contents.', async (t) => {
    const client = await resourceClient(t, true);
    const file = scratchFile(t);
    const data = `${t.uniqueId('contents')} — café 🌍`;
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data, encoding: 'utf-8' });
    assert.equal(await readFile(file.path, 'utf8'), data);
  }, spec),
  resourceTest('resourceWrite', 'default-truncate', 'resourceWrite overwrites and truncates an existing file by default.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'a longer original file');
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: 'new', encoding: 'utf-8' });
    assert.equal(await readFile(file.path, 'utf8'), 'new');
  }, spec),
  resourceTest('resourceWrite', 'truncate-position', 'resourceWrite truncates at the specified byte offset before writing.', async (t) => {
    const client = await resourceClient(t, true);
    const prefix = 'café';
    const file = await fixtureFile(t, `${prefix} old trailing contents`);
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: '!', encoding: 'utf-8', mode: 'truncate', position: Buffer.byteLength(prefix) });
    assert.equal(await readFile(file.path, 'utf8'), `${prefix}!`);
  }, spec),
  resourceTest('resourceWrite', 'append-eof', 'resourceWrite append mode writes at EOF when position is omitted.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'original');
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: ' appended', encoding: 'utf-8', mode: 'append' });
    assert.equal(await readFile(file.path, 'utf8'), 'original appended');
  }, spec),
  resourceTest('resourceWrite', 'append-position', 'resourceWrite append position counts bytes backwards from EOF and preserves the suffix.', async (t) => {
    const client = await resourceClient(t, true);
    const suffix = '🌍';
    const file = await fixtureFile(t, `hello${suffix}`);
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: ' ', encoding: 'utf-8', mode: 'append', position: Buffer.byteLength(suffix) });
    assert.equal(await readFile(file.path, 'utf8'), `hello ${suffix}`);
  }, spec),
  resourceTest('resourceWrite', 'insert-position', 'resourceWrite insert mode splices data at a byte offset without removing existing bytes.', async (t) => {
    const client = await resourceClient(t, true);
    const prefix = 'café';
    const file = await fixtureFile(t, `${prefix}tail`);
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: '-', encoding: 'utf-8', mode: 'insert', position: Buffer.byteLength(prefix) });
    assert.equal(await readFile(file.path, 'utf8'), `${prefix}-tail`);
  }, spec),
  resourceTest('resourceWrite', 'insert-default-position', 'resourceWrite insert mode defaults to inserting at the start of the file.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'tail');
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: 'head', encoding: 'utf-8', mode: 'insert' });
    assert.equal(await readFile(file.path, 'utf8'), 'headtail');
  }, spec),
  resourceTest('resourceWrite', 'base64-bytes', 'resourceWrite decodes base64 into the exact supplied binary bytes.', async (t) => {
    const client = await resourceClient(t, true);
    const file = scratchFile(t, 'binary', '.bin');
    const bytes = Buffer.from([0, 255, 128, 13, 10, 1, 254, 195, 40, 0]);
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: bytes.toString('base64'), encoding: 'base64' });
    assert.deepEqual(await readFile(file.path), bytes);
  }, spec),
  resourceTest('resourceWrite', 'create-only-existing', 'resourceWrite with createOnly returns AlreadyExists without modifying an existing file.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'original');
    await expectRpcError(client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: 'replacement', encoding: 'utf-8', createOnly: true }), AhpErrorCodes.AlreadyExists);
    assert.equal(await readFile(file.path, 'utf8'), 'original');
  }, spec),
  resourceTest('resourceWrite', 'create-only-new', 'resourceWrite with createOnly creates a file that does not exist.', async (t) => {
    const client = await resourceClient(t, true);
    const file = scratchFile(t);
    const data = t.uniqueId('contents');
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data, encoding: 'utf-8', createOnly: true });
    assert.equal(await readFile(file.path, 'utf8'), data);
  }, spec),
  resourceTest('resourceWrite', 'if-match-stale', 'resourceWrite rejects a stale ifMatch etag with Conflict without overwriting the file.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'original');
    const stale = await etag(t, client, file.uri, true);
    await writeFile(file.path, 'a substantially longer intervening update');
    await expectRpcError(client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: 'lost update', encoding: 'utf-8', ifMatch: stale }), AhpErrorCodes.Conflict);
    assert.equal(await readFile(file.path, 'utf8'), 'a substantially longer intervening update');
  }, spec),
  resourceTest('resourceWrite', 'if-match-current', 'resourceWrite accepts an ifMatch etag that matches the current file version.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, 'original');
    const current = await etag(t, client, file.uri, true);
    await client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data: 'replacement', encoding: 'utf-8', ifMatch: current });
    assert.equal(await readFile(file.path, 'utf8'), 'replacement');
  }, spec),
  resourceTest('resourceWrite', 'missing-parent', 'resourceWrite returns NotFound when the parent directory does not exist.', async (t) => {
    const client = await resourceClient(t, true);
    await expectRpcError(client.request('resourceWrite', { channel: rootChannel, uri: t.scratchUri(`${t.uniqueId('missing-parent')}/file.txt`), data: 'data', encoding: 'utf-8' }), AhpErrorCodes.NotFound);
  }, spec),
  resourceTest('resourceWrite', 'concurrent-append', 'resourceWrite append operations do not clobber concurrent appenders.', async (t) => {
    const client = await resourceClient(t, true);
    const file = await fixtureFile(t, '');
    const chunks = Array.from({ length: 8 }, (_, index) => `${t.uniqueId(`append-${index}`)}\n`);
    await Promise.all(chunks.map((data) => client.request('resourceWrite', { channel: rootChannel, uri: file.uri, data, encoding: 'utf-8', mode: 'append' })));
    const lines = (await readFile(file.path, 'utf8')).split('\n');
    assert.equal(lines.pop(), '');
    assert.deepEqual(lines.sort(), chunks.map((chunk) => chunk.trim()).sort());
  }, spec),
];
