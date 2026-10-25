import type { TestDefinition } from '../../harness/types.ts';
import { tests as ping } from './ping.ts';
import { tests as initialize } from './initialize.ts';
import { tests as advertisements } from './initialize-advertisements.ts';
import { tests as frames } from './frames.ts';
import { tests as jsonRpc } from './json-rpc.ts';
import { tests as actions } from './actions.ts';
import { tests as reconnect } from './reconnect.ts';

export const tests: TestDefinition[] = [
  ...ping,
  ...initialize,
  ...advertisements,
  ...frames,
  ...jsonRpc,
  ...actions,
  ...reconnect,
];
