import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { fixtureFile, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

export const tests: TestDefinition[] = (['write', 'mkdir', 'delete', 'move', 'copy'] as const).map((operation) => {
  const method = `resource${operation[0]!.toUpperCase()}${operation.slice(1)}`;
  return {
    ...resourceTest('results', `empty-success-${operation}`, `${method} returns an empty object on success.`, async (t) => {
      const client = await resourceClient(t, true);
      const source = await fixtureFile(t, 'source');
      const target = scratchFile(t, 'target', operation === 'mkdir' ? '' : '.txt');
      const params = operation === 'write'
        ? { channel: rootChannel, uri: target.uri, data: 'written', encoding: 'utf-8' }
        : operation === 'mkdir'
          ? { channel: rootChannel, uri: target.uri }
          : operation === 'delete'
            ? { channel: rootChannel, uri: source.uri }
            : { channel: rootChannel, source: source.uri, destination: target.uri };
      assert.deepEqual(await client.request(method, params), {});
    }, `/reference/common#${method.toLowerCase()}result`),
    feature: method,
  };
});
