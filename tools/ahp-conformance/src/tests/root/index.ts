import type { TestDefinition } from '../../harness/types.ts';
import { tests as config } from './config.ts';
import { tests as completions } from './completions.ts';
import { tests as resolveConfig } from './resolve-config.ts';
import { tests as subscriptions } from './subscriptions.ts';
import { tests as notifications } from './notifications.ts';

export const tests: TestDefinition[] = [
  ...config,
  ...completions,
  ...resolveConfig,
  ...subscriptions,
  ...notifications,
];
