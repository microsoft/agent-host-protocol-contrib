import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';

function assertRecord(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

export const tests: TestDefinition[] = [
  {
    id: 'transport/initialize/server-info-shape',
    area: 'Transport & lifecycle', feature: 'initialize',
    title: 'An advertised server identity contains a name and optional string version and title.',
    spec: '/reference/common#implementation',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const info = client.initializeResult.serverInfo;
      if (info === undefined) t.skip('InitializeResult does not advertise serverInfo.');
      assertRecord(info);
      assert.equal(typeof info.name, 'string');
      if (info.version !== undefined) assert.equal(typeof info.version, 'string');
      if (info.title !== undefined) assert.equal(typeof info.title, 'string');
    },
  },
  {
    id: 'transport/initialize/extension-metadata-object',
    area: 'Transport & lifecycle', feature: 'initialize',
    title: 'Advertised implementation extension metadata is an opaque object.',
    spec: '/reference/common#initializeresult',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const metadata = client.initializeResult._meta;
      if (metadata === undefined) t.skip('InitializeResult does not advertise extension metadata.');
      assertRecord(metadata);
    },
  },
  {
    id: 'transport/initialize/default-directory-uri',
    area: 'Transport & lifecycle', feature: 'initialize',
    title: 'An advertised default directory is a URI rather than a bare filesystem path.',
    spec: '/reference/common#initializeresult',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const directory = client.initializeResult.defaultDirectory;
      if (directory === undefined) return t.skip('InitializeResult does not advertise defaultDirectory.');
      assert.equal(typeof directory, 'string');
      assert.match(directory, /^[a-z][a-z0-9+.-]*:/i, 'defaultDirectory must include a URI scheme');
      assert.doesNotMatch(directory, /^[a-z]:[\\/]/i, 'a Windows filesystem path is not a directory URI');
      assert.doesNotMatch(directory, /[\u0000-\u0020\u007f]/, 'URI whitespace and controls must be escaped');
      assert.doesNotThrow(() => new URL(directory));
    },
  },
  {
    id: 'transport/initialize/completion-trigger-characters-shape',
    area: 'Transport & lifecycle', feature: 'initialize',
    title: 'Advertised completion trigger characters are an array of strings.',
    spec: '/reference/common#initializeresult',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      const characters = client.initializeResult.completionTriggerCharacters;
      if (characters === undefined) t.skip('InitializeResult does not advertise completionTriggerCharacters.');
      assert.ok(Array.isArray(characters));
      for (const character of characters) assert.equal(typeof character, 'string');
    },
  },
];
