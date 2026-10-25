import type { TestDefinition } from '../../harness/types.ts';
import { tests as lifecycle } from './lifecycle.ts';
import { tests as controls } from './controls.ts';
import { tests as serverOnly } from './server-only.ts';
import { tests as prefix } from './prefix.ts';

export const tests: TestDefinition[] = [...lifecycle, ...controls, ...serverOnly, ...prefix];
