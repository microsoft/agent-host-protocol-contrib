import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { resourceTest, rootChannel } from './helpers.ts';

export const tests: TestDefinition[] = [
  resourceTest('resourceRequest', 'grant-or-permission-denied', 'resourceRequest grants scratch access with an empty result or denies it with PermissionDenied.', async (t) => {
    const client = await t.client();
    let result: unknown;
    try {
      result = await client.request('resourceRequest', { channel: rootChannel, uri: t.scratchUri(), read: true, write: true });
    } catch (error) {
      assert.ok(error instanceof RpcError, 'resourceRequest denial must be a JSON-RPC error');
      assert.equal(error.code, AhpErrorCodes.PermissionDenied);
      return;
    }
    assert.deepEqual(result, {});
  }),
];
