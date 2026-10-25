import type { TestDefinition } from '../harness/types.ts';

export function matchesGlob(id: string, glob: string): boolean {
  let pattern = '^';
  for (let index = 0; index < glob.length; index++) {
    const character = glob[index];
    if (character === '*' && glob[index + 1] === '*') {
      index++;
      if (glob[index + 1] === '/') { pattern += '(?:.*/)?'; index++; }
      else pattern += '.*';
    } else if (character === '*') pattern += '[^/]*';
    else pattern += character.replace(/[\\^$.*+?()[\]{}|]/g, '\\$&');
  }
  return new RegExp(`${pattern}$`).test(id);
}

export function selectTests(tests: readonly TestDefinition[], include: readonly string[] = [], exclude: readonly string[] = [], filters: readonly string[] = []): TestDefinition[] {
  const ids = new Set<string>();
  for (const test of tests) {
    if (ids.has(test.id)) throw new Error(`Duplicate test id: ${test.id}`);
    ids.add(test.id);
  }
  const matches = (id: string, patterns: readonly string[]) => patterns.some((pattern) => matchesGlob(id, pattern));
  return tests.filter(({ id }) => (!include.length || matches(id, include)) && !matches(id, exclude) && (!filters.length || matches(id, filters)));
}
