import { writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { AhpErrorCodes, JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, ResourceReadResult, ResourceResolveResult } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, TestContext, TestDefinition, WaitOptions } from '../../harness/types.ts';

export const rootChannel = 'ahp-root://';

export function resourceTest(
  feature: string,
  name: string,
  title: string,
  run: TestDefinition['run'],
  spec = `/reference/common#${feature.toLowerCase()}`,
): TestDefinition {
  const slug = feature.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  return { id: `resources/${slug}/${name}`, area: 'Resources', feature, title, spec, requires: [requires.sharedFilesystem], run };
}

export async function resourceClient(t: TestContext, write = false): Promise<Client> {
  const client = await t.client();
  const request: Client['request'] = client.request.bind(client);
  let access: Promise<unknown> | undefined;
  // Permission negotiation is a fallback, never a prerequisite for another resource method.
  client.request = async <R = unknown>(method: string, params?: unknown, options?: WaitOptions): Promise<R> => {
    try {
      return await request<R>(method, params, options);
    } catch (error) {
      if (!(error instanceof RpcError) || error.code !== AhpErrorCodes.PermissionDenied
        || !(method.startsWith('resource') || method === 'createResourceWatch') || method === 'resourceRequest') throw error;
    }
    try {
      access ??= request('resourceRequest', { channel: rootChannel, uri: t.scratchUri(), read: true, write });
      await access;
    } catch (error) {
      if (error instanceof RpcError) {
        t.skip(`${method} denied scratch access and resourceRequest could not grant it (${error.code}: ${error.message}).`);
      }
      throw error;
    }
    try {
      return await request<R>(method, params, options);
    } catch (error) {
      if (error instanceof RpcError && error.code === AhpErrorCodes.PermissionDenied) {
        t.skip(`${method} still denied scratch access after resourceRequest.`);
      }
      throw error;
    }
  };
  return client;
}

export function scratchFile(t: TestContext, prefix = 'file', extension = '.txt') {
  const name = `${t.uniqueId(prefix)}${extension}`;
  return { name, path: join(t.scratchDir, name), uri: t.scratchUri(name) };
}

export async function fixtureFile(t: TestContext, contents: string | Uint8Array, extension = '.txt') {
  const file = scratchFile(t, 'fixture', extension);
  await writeFile(file.path, contents);
  return file;
}

export function decoded(result: ResourceReadResult): Buffer {
  assert.ok(result.encoding === 'utf-8' || result.encoding === 'base64', 'resourceRead must return a supported encoding');
  assert.equal(typeof result.data, 'string');
  if (result.contentType !== undefined) assert.equal(typeof result.contentType, 'string');
  return Buffer.from(result.data, result.encoding === 'base64' ? 'base64' : 'utf8');
}

export function resolve(client: Client, uri: string, followSymlinks?: boolean): Promise<ResourceResolveResult> {
  return client.request('resourceResolve', { channel: rootChannel, uri, followSymlinks });
}

export async function etag(t: TestContext, client: Client, uri: string, skipUnsupported = false): Promise<string> {
  let result: ResourceResolveResult;
  try {
    result = await resolve(client, uri);
  } catch (error) {
    if (skipUnsupported && error instanceof RpcError && error.code === JsonRpcErrorCodes.MethodNotFound) {
      t.skip('The host does not support resourceResolve, which is needed to obtain an optional etag.');
    }
    throw error;
  }
  if (result.etag === undefined) t.skip('This resource provider does not expose optional etags.');
  assert.equal(typeof result.etag, 'string');
  return result.etag;
}

export async function expectRpcFailure(promise: Promise<unknown>): Promise<void> {
  try {
    await promise;
  } catch (error) {
    if (!(error instanceof RpcError)) throw error;
    return;
  }
  assert.fail('Expected a JSON-RPC error.');
}

export function hasChange(envelope: ActionEnvelope, uri: string, type?: 'added' | 'updated' | 'deleted'): boolean {
  return envelope.action.type === 'resourceWatch/changed'
    && envelope.action.changes.items.some((change) => change.uri === uri && (type === undefined || change.type === type));
}
