import type { TestDefinition } from '../../harness/types.ts';
import { tests as clientToolTests } from './client-tools.ts';

export const tests: TestDefinition[] = [...clientToolTests];
