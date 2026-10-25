import type { TestDefinition } from '../../harness/types.ts';
import { tests as chatTests } from './chats.ts';
import { tests as fetchTurnsTests } from './fetch-turns.ts';
import { tests as forkTests } from './forks.ts';
import { tests as pendingTests } from './pending.ts';
import { tests as turnTests } from './turns.ts';
import { tests as projectionTests } from './projections.ts';
import { tests as validationTests } from './validation.ts';
import { tests as moveValidationTests } from './move-validation.ts';
import { tests as attachmentTests } from './attachments.ts';
import { tests as workingDirectoryTests } from './working-directories.ts';
import { tests as turnContractTests } from './turn-contracts.ts';
import { tests as resumeTests } from './resume.ts';

export const tests: TestDefinition[] = [
  ...turnTests,
  ...fetchTurnsTests,
  ...chatTests,
  ...forkTests,
  ...pendingTests,
  ...projectionTests,
  ...validationTests,
  ...moveValidationTests,
  ...attachmentTests,
  ...workingDirectoryTests,
  ...turnContractTests,
  ...resumeTests,
];
