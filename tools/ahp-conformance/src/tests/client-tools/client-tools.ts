import type { ActionEnvelope, ChatState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { LooseStateAction, TestDefinition } from '../../harness/types.ts';
import { assertCallMatchesSnapshot, callIn, clientTool, content, dispatchAccepted, dispatchEcho, finishTool, normalized } from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'tools/client-tools/owner-completion', area: 'Tools & permissions', feature: 'clientTools',
    title: 'The owning client completes its contributed tool and unblocks the model turn.',
    spec: '/reference/chat#chattoolcallcompleteaction', requires: [requires.model],
    async run(t) {
      const fixture = await clientTool(t, true);
      const { owner, observer, session, turnId, toolCallId, reply } = fixture;
      const before = owner.lastServerSeq;
      await owner.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === session.chatUri && event.serverSeq > before &&
        event.rejectionReason === undefined && (event.action.type === 'chat/toolCallComplete' || event.action.type === 'chat/turnComplete') && event.action.turnId === turnId, 350);
      const { result, echo } = await finishTool(fixture, t.uniqueId('owner-result'));
      assert.equal(echo.origin?.clientId, owner.clientId);
      for (const client of [owner, observer]) {
        const call = callIn(client.state<ChatState>(session.chatUri), turnId, toolCallId);
        assert.ok(call?.status === 'completed', 'client-completed tool must be retained as completed');
        assert.equal(call.success, true);
        assert.deepEqual(normalized(call.content), normalized(result.content));
        assert.deepEqual(normalized(call.structuredContent), normalized(result.structuredContent));
      }
      assert.ok(session.responseText(turnId).trim().includes(reply), 'model did not continue after the client result');
      await assertCallMatchesSnapshot(t, owner, session.chatUri, turnId, toolCallId);
    },
  },
  {
    id: 'tools/client-tools/content-changed-replaces-running-content', area: 'Tools & permissions', feature: 'clientTools',
    title: 'Owner content updates replace the running client tool content on every subscriber.',
    spec: '/reference/chat#chattoolcallcontentchangedaction', requires: [requires.model],
    async run(t) {
      const fixture = await clientTool(t);
      const { owner, observer, session, turnId, toolCallId } = fixture;
      for (const text of [t.uniqueId('first-partial'), t.uniqueId('replacement')]) {
        const expected = content(text);
        const echo = await dispatchAccepted(owner, session.chatUri, { type: 'chat/toolCallContentChanged', turnId, toolCallId, content: expected });
        await observer.waitForAction(session.chatUri, (e) => e.serverSeq === echo.serverSeq);
        for (const client of [owner, observer]) {
          const call = callIn(client.state<ChatState>(session.chatUri), turnId, toolCallId);
          assert.ok(call?.status === 'running');
          assert.deepEqual(normalized(call.content), normalized(expected), 'running content must be replaced, not appended');
        }
        await assertCallMatchesSnapshot(t, owner, session.chatUri, turnId, toolCallId);
      }
      await finishTool(fixture, t.uniqueId('finished'));
    },
  },
  ...(['completion', 'content'] as const).map((kind): TestDefinition => ({
    id: `tools/client-tools/non-owner-${kind}-rejected`, area: 'Tools & permissions', feature: 'clientTools',
    title: `SHOULD: A non-owner client cannot ${kind === 'completion' ? 'complete' : 'change the content of'} another client's tool invocation.`,
    spec: kind === 'completion' ? '/reference/chat#chattoolcallcompleteaction' : '/reference/chat#chattoolcallcontentchangedaction',
    requires: [requires.model],
    async run(t) {
      const fixture = await clientTool(t);
      const { owner, observer, session, turnId, toolCallId } = fixture;
      const before = callIn(owner.state<ChatState>(session.chatUri), turnId, toolCallId);
      const action: LooseStateAction = kind === 'completion'
        ? { type: 'chat/toolCallComplete', turnId, toolCallId, result: { success: true, pastTenseMessage: 'Spoofed completion', content: content('not-owner') } }
        : { type: 'chat/toolCallContentChanged', turnId, toolCallId, content: content('not-owner') };
      const echo = await dispatchEcho(observer, session.chatUri, action);
      assert.ok(typeof echo.rejectionReason === 'string' && echo.rejectionReason.length > 0, 'ownership violation must be rejected');
      await owner.request('ping', { channel: 'ahp-root://' });
      assert.deepEqual(normalized(callIn(owner.state<ChatState>(session.chatUri), turnId, toolCallId)), normalized(before));
      await assertCallMatchesSnapshot(t, owner, session.chatUri, turnId, toolCallId);
      await finishTool(fixture, t.uniqueId('real-owner'));
    },
  })),
];
