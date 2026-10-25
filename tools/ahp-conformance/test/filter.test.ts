import assert from 'node:assert/strict';
import test from 'node:test';
import { matchesGlob, selectTests } from '../src/runner/filter.ts';
import type { TestDefinition } from '../src/harness/types.ts';

const definition = (id: string): TestDefinition => ({ id, title: id, area: 'Transport & lifecycle', feature: 'example', async run() {} });

test('glob stars stay within a segment; double stars cross segments', () => {
  assert.ok(matchesGlob('transport/ping/success', 'transport/**'));
  assert.ok(matchesGlob('transport/ping/success', 'transport/*/success'));
  assert.ok(matchesGlob('transport/ping/success', '**/success'));
  assert.ok(matchesGlob('success', '**/success'));
  assert.ok(matchesGlob('transport/success', 'transport/**/success'));
  assert.equal(matchesGlob('transport/ping/success', 'transport/*'), false);
  assert.equal(matchesGlob('transport/ping/success', 'sessions/**'), false);
  assert.ok(matchesGlob('tools/a.b/(x)', 'tools/a.b/(x)'));
  assert.equal(matchesGlob('tools/axb/(x)', 'tools/a.b/(x)'), false);
});

test('filtering intersects include/CLI filters and subtracts exclusions', () => {
  const tests = ['transport/ping/success', 'transport/ping/error', 'sessions/create/ready'].map(definition);
  assert.equal(selectTests(tests).length, 3);
  assert.deepEqual(selectTests(tests, ['transport/**'], ['**/error'], ['**/success']).map((value) => value.id), ['transport/ping/success']);
  assert.deepEqual(selectTests(tests, [], [], ['**/error', 'sessions/**']).map((value) => value.id), ['transport/ping/error', 'sessions/create/ready']);
  assert.throws(() => selectTests([tests[0], tests[0]]), /Duplicate test id/);
});
