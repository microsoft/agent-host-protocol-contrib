import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { readTurn, toolCalls, toolTest } from './helpers.ts';

export const tests: TestDefinition[] = [
  toolTest({
    id: 'tools/tool-calls/read-file-recorded', feature: 'toolCalls',
    title: 'A read-file turn records a tool call in chat state.',
    spec: '/reference/chat#toolcallresponsepart',
    async run(t) {
      const { turn } = await readTurn(t);
      assert(toolCalls(turn).length > 0, 'no tool call was recorded');
    },
  }),
  toolTest({
    id: 'tools/tool-calls/lifecycle-order', feature: 'toolCalls',
    title: 'Tool-call lifecycle actions follow the specified state transitions.',
    spec: '/guide/state-model#tool-call-lifecycle',
    async run(t) {
      const { client, session, turnId } = await readTurn(t);
      const statuses = new Map<string, string>();
      for (const { action, rejectionReason } of client.actions(session.chatUri)) {
        if (rejectionReason !== undefined || !('turnId' in action) || action.turnId !== turnId || !('toolCallId' in action)) continue;
        const previous = statuses.get(action.toolCallId);
        let next = previous;
        switch (action.type) {
          case 'chat/toolCallStart':
            assert.equal(previous, undefined, 'tool call was started more than once');
            next = 'streaming';
            break;
          case 'chat/toolCallDelta':
            assert.equal(previous, 'streaming');
            break;
          case 'chat/toolCallReady':
            assert(previous === 'streaming' || previous === 'running' || previous === 'pending-confirmation', `ready from ${previous}`);
            next = action.confirmed !== undefined ? 'running' : 'pending-confirmation';
            break;
          case 'chat/toolCallConfirmed':
            assert.equal(previous, 'pending-confirmation');
            next = action.approved ? 'running' : 'cancelled';
            break;
          case 'chat/toolCallComplete':
            assert(previous === 'running' || previous === 'pending-confirmation' || previous === 'auth-required', `complete from ${previous}`);
            if (previous === 'auth-required') assert.equal(action.result.success, false);
            next = previous !== 'auth-required' && action.requiresResultConfirmation ? 'pending-result-confirmation' : 'completed';
            break;
          case 'chat/toolCallResultConfirmed':
            assert.equal(previous, 'pending-result-confirmation');
            next = action.approved ? 'completed' : 'cancelled';
            break;
          case 'chat/toolCallAuthRequired':
            assert.equal(previous, 'running');
            next = 'auth-required';
            break;
          case 'chat/toolCallAuthResolved':
            assert.equal(previous, 'auth-required');
            next = 'running';
            break;
        }
        if (next !== undefined) statuses.set(action.toolCallId, next);
      }
      assert(statuses.size > 0, 'no tool-call lifecycle was observed');
    },
  }),
  toolTest({
    id: 'tools/tool-calls/completed-state', feature: 'toolCalls',
    title: 'A successful read-file turn retains a completed tool call.',
    spec: '/reference/chat#toolcallcompletedstate',
    async run(t) {
      const { turn } = await readTurn(t);
      assert(toolCalls(turn).some((call) => call.status === 'completed' && call.success), 'no successful completed tool call');
    },
  }),
  toolTest({
    id: 'tools/tool-calls/result-content-recorded', feature: 'toolCalls',
    title: 'Tool result content supplied at completion is retained in chat state.',
    spec: '/reference/chat#toolcallresult',
    async run(t) {
      const { client, session, turnId, turn } = await readTurn(t);
      let checked = 0;
      for (const { action, rejectionReason } of client.actions(session.chatUri)) {
        if (rejectionReason !== undefined || action.type !== 'chat/toolCallComplete' || action.turnId !== turnId) continue;
        if (action.result.content === undefined && action.result.structuredContent === undefined) continue;
        const call = toolCalls(turn).find((call) => call.toolCallId === action.toolCallId);
        assert(call?.status === 'completed', 'completed result missing from chat state');
        if (action.result.content !== undefined) assert.deepEqual(call.content, action.result.content);
        if (action.result.structuredContent !== undefined) assert.deepEqual(call.structuredContent, action.result.structuredContent);
        checked++;
      }
      if (!checked) t.skip('host did not supply optional tool result content');
    },
  }),
  toolTest({
    id: 'tools/tool-calls/final-reply-includes-file-token', feature: 'toolCalls',
    title: 'The assistant reply after a file read includes the file token.',
    spec: '/specification/chat-channel#active-chat',
    async run(t) {
      const { session, turnId, token } = await readTurn(t);
      assert(session.responseText(turnId).trim().includes(token), 'assistant reply omitted the file token');
    },
  }),
  toolTest({
    id: 'tools/tool-calls/unique-identifiers', feature: 'toolCalls',
    title: 'Tool-call identifiers are unique within a turn.',
    spec: '/reference/chat#toolcallstate',
    async run(t) {
      const { client, session, turnId, turn } = await readTurn(t);
      const started = client.actions(session.chatUri).flatMap(({ action, rejectionReason }) =>
        rejectionReason === undefined && action.type === 'chat/toolCallStart' && action.turnId === turnId ? [action.toolCallId] : []);
      assert(started.length > 0, 'no tool calls were started');
      assert.equal(new Set(started).size, started.length);
      const recorded = toolCalls(turn).map((call) => call.toolCallId);
      assert.equal(new Set(recorded).size, recorded.length);
    },
  }),
  toolTest({
    id: 'tools/tool-calls/mirror-matches-snapshot', feature: 'toolCalls',
    title: 'The chat mirror matches a fresh snapshot after a tool turn.',
    spec: '/guide/actions#reducers',
    async run(t) {
      const { client, session } = await readTurn(t);
      await client.assertMirrorMatchesSnapshot(session.chatUri);
    },
  }),
];
