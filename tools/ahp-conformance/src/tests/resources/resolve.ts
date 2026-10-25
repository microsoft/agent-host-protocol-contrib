import { mkdir, realpath, symlink, writeFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { TestContext, TestDefinition } from '../../harness/types.ts';
import { etag, fixtureFile, resolve, resourceClient, resourceTest, scratchFile } from './helpers.ts';

const spec = '/reference/common#resourceresolve';

async function linkedFile(t: TestContext) {
  const file = await fixtureFile(t, 'symlink target');
  const link = scratchFile(t, 'link');
  try {
    await symlink(file.name, link.path);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === 'EPERM' || code === 'EACCES' || code === 'ENOTSUP') t.skip('The shared filesystem does not permit this test to create symlinks.');
    throw error;
  }
  return { file, link };
}

export const tests: TestDefinition[] = [
  resourceTest('resourceResolve', 'non-symlink-uri', 'resourceResolve preserves the URI of a canonical file that does not traverse a symlink.', async (t) => {
    const client = await resourceClient(t);
    const file = await fixtureFile(t, 'regular resource');
    const uri = pathToFileURL(await realpath(file.path)).href;
    assert.equal((await resolve(client, uri)).uri, uri);
  }, spec),
  resourceTest('resourceResolve', 'file-metadata', 'resourceResolve returns file metadata consistent with the shared filesystem.', async (t) => {
    const client = await resourceClient(t);
    const contents = 'café 🌍';
    const file = await fixtureFile(t, contents);
    const result = await resolve(client, file.uri);
    assert.equal(result.type, 'file');
    if (result.size !== undefined) assert.equal(result.size, Buffer.byteLength(contents));
    if (result.mtime !== undefined) assert.ok(Number.isFinite(Date.parse(result.mtime)), 'mtime must be an ISO 8601 timestamp');
    if (result.ctime !== undefined) assert.ok(Number.isFinite(Date.parse(result.ctime)), 'ctime must be an ISO 8601 timestamp');
    if (result.contentType !== undefined) assert.equal(typeof result.contentType, 'string');
    if (result.etag !== undefined) assert.equal(typeof result.etag, 'string');
  }, spec),
  resourceTest('resourceResolve', 'directory-type', 'resourceResolve identifies a directory as a directory.', async (t) => {
    const client = await resourceClient(t);
    const directory = scratchFile(t, 'directory', '');
    await mkdir(directory.path);
    assert.equal((await resolve(client, directory.uri)).type, 'directory');
  }, spec),
  resourceTest('resourceResolve', 'missing-resource', 'resourceResolve returns NotFound rather than a sentinel for a missing resource.', async (t) => {
    const client = await resourceClient(t);
    await expectRpcError(resolve(client, scratchFile(t, 'missing').uri), AhpErrorCodes.NotFound);
  }, spec),
  resourceTest('resourceResolve', 'follow-symlink-default', 'resourceResolve follows symlinks by default and returns the canonical target URI.', async (t) => {
    const client = await resourceClient(t);
    const { file, link } = await linkedFile(t);
    const result = await resolve(client, link.uri);
    assert.equal(result.uri, pathToFileURL(await realpath(file.path)).href);
    assert.equal(result.type, 'file');
  }, spec),
  resourceTest('resourceResolve', 'symlink-no-follow', 'resourceResolve with followSymlinks disabled reports the link itself.', async (t) => {
    const client = await resourceClient(t);
    const { link } = await linkedFile(t);
    const result = await resolve(client, link.uri, false);
    assert.equal(result.type, 'symlink');
    assert.equal(result.uri, link.uri);
  }, spec),
  resourceTest('resourceResolve', 'etag-changes', 'resourceResolve returns a different etag after a file’s content changes.', async (t) => {
    const client = await resourceClient(t);
    const file = await fixtureFile(t, 'before');
    const before = await etag(t, client, file.uri);
    await writeFile(file.path, 'after with a different byte length');
    assert.notEqual(await etag(t, client, file.uri), before);
  }, spec),
];
