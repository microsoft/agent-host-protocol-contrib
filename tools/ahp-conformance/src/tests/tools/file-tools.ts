import { readFile } from 'node:fs/promises';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { toolTest, writeTurn } from './helpers.ts';

export const tests: TestDefinition[] = [
  toolTest({
    id: 'tools/file-tools/write-exact-contents', feature: 'fileTools',
    title: 'A write-file turn creates the file with the requested contents.',
    spec: '/specification/chat-channel#active-chat',
    async run(t) {
      const { filePath, contents } = await writeTurn(t);
      assert.equal(await readFile(filePath, 'utf8'), contents);
    },
  }),
];
