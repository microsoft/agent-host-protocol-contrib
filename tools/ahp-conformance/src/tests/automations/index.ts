import type { TestDefinition } from '../../harness/types.ts';
import { tests as catalogue } from './catalogue.ts';
import { tests as runs } from './runs.ts';

export const tests: TestDefinition[] = [...catalogue, ...runs];
