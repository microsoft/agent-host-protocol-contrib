import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { assertRecord } from './helpers.ts';

function assertAttachment(value: unknown): void {
  assertRecord(value);
  assert.equal(typeof value.type, 'string');
  assert.equal(typeof value.label, 'string');
  if (value.displayKind !== undefined) assert.equal(typeof value.displayKind, 'string');
  if (value._meta !== undefined) assertRecord(value._meta);
  switch (value.type) {
    case 'simple':
      if (value.modelRepresentation !== undefined) assert.equal(typeof value.modelRepresentation, 'string');
      break;
    case 'embeddedResource':
      assert.equal(typeof value.data, 'string');
      assert.equal(typeof value.contentType, 'string');
      break;
    case 'resource':
      assert.equal(typeof value.uri, 'string');
      break;
    case 'annotations':
      assert.equal(typeof value.resource, 'string');
      if (value.annotationIds !== undefined) {
        assert.ok(Array.isArray(value.annotationIds));
        for (const id of value.annotationIds) assert.equal(typeof id, 'string');
      }
      break;
    case 'chat':
      assert.equal(typeof value.resource, 'string');
      if (value.endTurn !== undefined) assert.equal(typeof value.endTurn, 'string');
      break;
  }
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/completions/result-contract',
    area: 'Root & sessions', feature: 'completions',
    title: 'Message completions return items with insertion text and typed attachments.',
    spec: '/reference/session#completions',
    requires: [requires.completionTriggerCharacters],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const text = t.profile.initialize.completionTriggerCharacters![0]!;
      const result = await client.request('completions', {
        channel: session.chatUri, kind: 'userMessage', text, offset: text.length,
      });
      assertRecord(result);
      assert.ok(Array.isArray(result.items));
      for (const item of result.items) {
        assertRecord(item);
        assert.equal(typeof item.insertText, 'string');
        assertAttachment(item.attachment);
      }
    },
  },
];
