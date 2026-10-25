import type { TestDefinition } from '../../harness/types.ts';
import { tests as channels } from './channels.ts';

export const tests: TestDefinition[] = [...channels];
