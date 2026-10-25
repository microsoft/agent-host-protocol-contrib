import type { TestDefinition } from '../../harness/types.ts';
import { tests as protectedResources } from './protected-resources.ts';
import { tests as authenticate } from './authenticate.ts';
import { tests as createSession } from './create-session.ts';
import { tests as authRequired } from './auth-required.ts';

export const tests: TestDefinition[] = [...protectedResources, ...authenticate, ...createSession, ...authRequired];
