import type { TestDefinition } from '../../harness/types.ts';
import { tests as toolCalls } from './tool-calls.ts';
import { tests as fileTools } from './file-tools.ts';
import { tests as confirmations } from './confirmations.ts';
import { tests as cancellation } from './cancellation.ts';
import { tests as workingDirectories } from './working-directories.ts';

export const tests: TestDefinition[] = [
  ...toolCalls,
  ...fileTools,
  ...confirmations,
  ...cancellation,
  ...workingDirectories,
];
