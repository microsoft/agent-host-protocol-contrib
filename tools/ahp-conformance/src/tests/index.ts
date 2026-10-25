import type { TestDefinition } from '../harness/types.ts';
import { tests as auth } from './auth/index.ts';
import { tests as automations } from './automations/index.ts';
import { tests as chats } from './chats/index.ts';
import { tests as clientTools } from './client-tools/index.ts';
import { tests as proposed } from './proposed/index.ts';
import { tests as resources } from './resources/index.ts';
import { tests as root } from './root/index.ts';
import { tests as sessions } from './sessions/index.ts';
import { tests as telemetry } from './telemetry/index.ts';
import { tests as terminals } from './terminals/index.ts';
import { tests as tools } from './tools/index.ts';
import { tests as transport } from './transport/index.ts';

export const allTests: TestDefinition[] = [
  ...transport, ...root, ...sessions, ...chats, ...resources, ...tools, ...clientTools, ...auth,
  ...terminals, ...automations, ...telemetry, ...proposed,
];
