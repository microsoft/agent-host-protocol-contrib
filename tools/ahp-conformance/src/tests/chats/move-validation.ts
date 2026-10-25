import type { ChatState, ListSessionsResult, SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { chatState, createChat, normalized, snapshotState } from './helpers.ts';

export const tests: TestDefinition[] = (['unknown-source', 'non-movable-source', 'unknown-destination', 'unknown-anchor', 'foreign-anchor'] as const).map((kind): TestDefinition => ({
  id: `chats/move-chat/${kind}-preserves-state`, area: 'Chats & turns', feature: 'moveChat',
  title: `A move with ${kind.replaceAll('-', ' ')} is rejected without changing observable state.`,
  spec: '/specification/chat-channel#validation',
  requires: kind === 'unknown-source' || kind === 'non-movable-source' ? [] : [requires.multipleChats],
  async run(t) {
    const client = await t.client();
    const source = await t.session(client);
    const target = await t.session(client);
    await source.ready();
    await target.ready();
    let chat = source.chatUri;
    if (kind === 'unknown-source') chat = `ahp-chat:/${t.uniqueId('unknown')}`;
    else if (kind !== 'non-movable-source') {
      chat = await createChat(t, client, source);
      if (chatState(client, chat).movable !== true) t.skip('The host did not advertise a movable source chat.');
    }
    const beforeSource = await snapshotState<SessionState>(t, source.sessionUri);
    const beforeTarget = await snapshotState<SessionState>(t, target.sessionUri);
    const beforeChat = kind === 'unknown-source' ? undefined : await snapshotState<ChatState>(t, chat);
    const beforeRoot = await client.request<ListSessionsResult>('listSessions', { channel: 'ahp-root://' });
    const destination = {
      kind: 'session',
      session: kind === 'unknown-destination' ? `ahp-session:/${t.uniqueId('unknown')}` : target.sessionUri,
      ...(kind === 'unknown-anchor' ? { after: `ahp-chat:/${t.uniqueId('unknown')}` } : {}),
      ...(kind === 'foreign-anchor' ? { after: source.chatUri } : {}),
    };
    let caught: unknown;
    try { await client.request('moveChat', { channel: chat, destination }); }
    catch (error) { caught = error; }
    if (caught instanceof RpcError && caught.code === -32601 && (kind === 'unknown-source' || kind === 'non-movable-source')) {
      t.skip('The host has no moveChat method and this case has no advertised movable source.');
    }
    assert.ok(caught instanceof RpcError, 'invalid move must return a JSON-RPC error');
    assert.notEqual(caught.code, -32601, 'a host advertising movable:true must implement moveChat');
    assert.deepEqual(normalized(await snapshotState<SessionState>(t, source.sessionUri)), normalized(beforeSource));
    assert.deepEqual(normalized(await snapshotState<SessionState>(t, target.sessionUri)), normalized(beforeTarget));
    if (beforeChat) assert.deepEqual(normalized(await snapshotState<ChatState>(t, chat)), normalized(beforeChat));
    const afterRoot = await client.request<ListSessionsResult>('listSessions', { channel: 'ahp-root://' });
    for (const uri of [source.sessionUri, target.sessionUri]) {
      const before = beforeRoot.items.find((entry) => entry.resource === uri);
      if (before) assert.deepEqual(normalized(afterRoot.items.find((entry) => entry.resource === uri)), normalized(before), 'rejected move changed root session summary');
    }
  },
}));
