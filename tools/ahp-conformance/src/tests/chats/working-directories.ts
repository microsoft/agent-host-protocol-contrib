import { mkdir, rm } from 'node:fs/promises';
import { join } from 'node:path';
import type { ChatState, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { normalized, rejected, snapshotState } from './helpers.ts';

export const tests: TestDefinition[] = (['creation', 'action'] as const).map((kind): TestDefinition => ({
  id: `chats/working-directories/${kind}-outside-session-rejected`, area: 'Chats & turns', feature: 'workingDirectories',
  title: `A chat working-directory ${kind} rejects a directory outside the owning session set.`,
  spec: kind === 'creation' ? '/reference/chat#createchatparams' : '/guide/state-model#managing-the-subset-after-creation',
  requires: kind === 'creation' ? [requires.multipleWorkingDirectories, requires.multipleChats] : [requires.multipleWorkingDirectories],
  async run(t) {
    const client = await t.client();
    const session = await t.session(client);
    await session.ready();
    const folder = t.uniqueId('outside-subset');
    const path = join(t.scratchDir, folder);
    await mkdir(path);
    t.defer(() => rm(path, { recursive: true, force: true }));
    const directory = t.scratchUri(folder);
    const beforeOwner = await snapshotState<SessionState>(t, session.sessionUri);
    assert.ok(!beforeOwner.workingDirectories?.some((entry) => entry === directory), 'test directory is unexpectedly already in the session set');
    const beforeChat = await snapshotState<ChatState>(t, session.chatUri);
    if (kind === 'creation') {
      await assert.rejects(client.request('createChat', {
        channel: session.sessionUri, chat: `ahp-chat:/${t.uniqueId('invalid-subset')}`, workingDirectories: [directory],
      }), RpcError);
    } else {
      await rejected(client, session.chatUri, { type: 'chat/workingDirectorySet', directory });
    }
    assert.deepEqual(normalized(await snapshotState<SessionState>(t, session.sessionUri)), normalized(beforeOwner));
    assert.deepEqual(normalized(await snapshotState<ChatState>(t, session.chatUri)), normalized(beforeChat));
  },
}));
