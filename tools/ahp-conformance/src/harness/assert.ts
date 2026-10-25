import assert from 'node:assert/strict';
import { RpcError } from './types.ts';

export { assert };

/** Asserts that `promise` rejects with an `RpcError` carrying `code`, and returns that error. */
export async function expectRpcError(promise: Promise<unknown>, code: number): Promise<RpcError> {
  let error: unknown;
  try {
    const result = await promise;
    assert.fail(`expected JSON-RPC error ${code}, got result ${JSON.stringify(result)}`);
  } catch (caught) {
    error = caught;
  }
  if (!(error instanceof RpcError)) throw error;
  assert.equal(error.code, code, `expected JSON-RPC error ${code}, got ${error.code}: ${error.message}`);
  return error;
}
