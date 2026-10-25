import type { TestDefinition } from '../../harness/types.ts';
import { tests as rootSnapshot } from './root-snapshot.ts';
import { tests as createSession } from './create-session.ts';
import { tests as sessionSnapshot } from './session-snapshot.ts';
import { tests as subscribe } from './subscribe.ts';
import { tests as unsubscribe } from './unsubscribe.ts';
import { tests as listSessions } from './list-sessions.ts';
import { tests as rootNotifications } from './root-notifications.ts';
import { tests as sessionActions } from './session-actions.ts';
import { tests as resolveSessionConfig } from './resolve-session-config.ts';
import { tests as sessionConfigCompletions } from './session-config-completions.ts';
import { tests as sessionConfig } from './session-config.ts';
import { tests as disposeSession } from './dispose-session.ts';
import { tests as chatCatalog } from './chat-catalog.ts';
import { tests as activeClients } from './active-clients.ts';
import { tests as inputNeeded } from './input-needed.ts';
import { tests as summary } from './summary.ts';
import { tests as changesets } from './changesets.ts';

export const tests: TestDefinition[] = [
  ...rootSnapshot,
  ...createSession,
  ...sessionSnapshot,
  ...subscribe,
  ...unsubscribe,
  ...listSessions,
  ...rootNotifications,
  ...sessionActions,
  ...resolveSessionConfig,
  ...sessionConfigCompletions,
  ...sessionConfig,
  ...disposeSession,
  ...chatCatalog,
  ...activeClients,
  ...inputNeeded,
  ...summary,
  ...changesets,
];
