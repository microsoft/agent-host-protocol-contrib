import type { TestDefinition } from '../../harness/types.ts';
import { tests as read } from './read.ts';
import { tests as write } from './write.ts';
import { tests as list } from './list.ts';
import { tests as mkdir } from './mkdir.ts';
import { tests as deleteTests } from './delete.ts';
import { tests as move } from './move.ts';
import { tests as copy } from './copy.ts';
import { tests as resolve } from './resolve.ts';
import { tests as request } from './request.ts';
import { tests as watch } from './watch.ts';
import { tests as results } from './results.ts';
import { tests as contentRef } from './content-ref.ts';

export const tests: TestDefinition[] = [
  ...read,
  ...write,
  ...list,
  ...mkdir,
  ...deleteTests,
  ...move,
  ...copy,
  ...resolve,
  ...request,
  ...watch,
  ...results,
  ...contentRef,
];
