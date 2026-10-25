import type { ChatState, SessionState, ToolCallState, ToolResultContent, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import type { Client, LooseStateAction, TestContext } from '../../harness/types.ts';

export function normalized<T>(value: T): T {
  return value === undefined ? value : JSON.parse(JSON.stringify(value)) as T;
}

export async function dispatchEcho(client: Client, channel: string, action: LooseStateAction) {
  const clientSeq = client.dispatch(channel, action);
  return client.waitForAction(channel, (event) => event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
}

export async function dispatchAccepted(client: Client, channel: string, action: LooseStateAction) {
  const echo = await dispatchEcho(client, channel, action);
  assert.equal(echo.rejectionReason, undefined, 'owner client-tool action must be accepted');
  return echo;
}

export function callIn(state: ChatState | undefined, turnId: string, toolCallId?: string): ToolCallState | undefined {
  const turn = state?.activeTurn?.id === turnId ? state.activeTurn : state?.turns.find((entry) => entry.id === turnId);
  return turn?.responseParts.flatMap((part) => part.kind === 'toolCall' ? [part.toolCall] : [])
    .find((call) => toolCallId === undefined ? call.contributor?.kind === 'client' : call.toolCallId === toolCallId);
}

export function content(text: string): ToolResultContent[] {
  return [{ type: 'text', text }] as ToolResultContent[];
}

export async function clientTool(t: TestContext, primary = false) {
  const owner = await t.client({ label: 'tool-owner' });
  const session = await t.session(owner);
  await session.ready();
  const name = 'ahp_conformance_echo';
  const registered = await dispatchEcho(owner, session.sessionUri, {
    type: 'session/activeClientSet',
    activeClient: {
      clientId: owner.clientId, displayName: 'Conformance tool owner',
      tools: [{ name, description: 'Returns the supplied text without side effects; implemented by the active client.',
        inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] },
        annotations: { readOnlyHint: true, destructiveHint: false } }],
    },
  });
  if (registered.rejectionReason && !primary) t.skip('Client tool registration was rejected; owner-completion tests this prerequisite.');
  assert.equal(registered.rejectionReason, undefined, 'active client tool registration must be accepted');
  assert.ok(owner.state<SessionState>(session.sessionUri)?.activeClients.some((entry) => entry.clientId === owner.clientId && entry.tools.some((tool) => tool.name === name)), 'registered client tool missing from session registry');
  const observer = await t.client({ label: 'tool-observer' });
  await observer.subscribe(session.sessionUri);
  await observer.subscribe(session.chatUri);
  const reply = t.uniqueId('client-tool-finished');
  const turnId = session.startTurn(prompts.callTool(name, { text: t.uniqueId('tool-argument') }, reply));
  const state = await owner.waitForState<ChatState>(session.chatUri, (chat) => {
    const call = callIn(chat, turnId);
    return chat.turns.some((turn) => turn.id === turnId) || (call !== undefined && call.status !== 'streaming');
  });
  const call = callIn(state, turnId);
  if ((!call || call.contributor?.kind !== 'client' || call.contributor.clientId !== owner.clientId) && !primary) {
    t.skip('The model turn did not produce an owner-contributed client tool; owner-completion tests this prerequisite.');
  }
  assert.ok(call, 'the model did not invoke the registered client tool');
  assert.equal(call.contributor?.kind, 'client', 'registered tool must be client-contributed');
  assert.ok(call.contributor?.kind === 'client' && call.contributor.clientId === owner.clientId, 'execution ownership must identify the registering client');
  if (call.status === 'pending-confirmation') {
    await dispatchAccepted(owner, session.chatUri, {
      type: 'chat/toolCallConfirmed', turnId, toolCallId: call.toolCallId, approved: true, confirmed: 'user-action',
    } as LooseStateAction);
  }
  const current = callIn(owner.state<ChatState>(session.chatUri), turnId, call.toolCallId);
  assert.equal(current?.status, 'running', 'client invocation must wait in running state for its owner');
  await observer.waitForState<ChatState>(session.chatUri, (chat) => callIn(chat, turnId, call.toolCallId)?.status === 'running');
  return { owner, observer, session, turnId, toolCallId: call.toolCallId, reply };
}

export async function finishTool(fixture: Awaited<ReturnType<typeof clientTool>>, resultText: string) {
  const { owner, observer, session, turnId, toolCallId } = fixture;
  const result = { success: true, pastTenseMessage: 'Returned client text', content: content(resultText), structuredContent: { text: resultText } };
  const echo = await dispatchAccepted(owner, session.chatUri, { type: 'chat/toolCallComplete', turnId, toolCallId, result });
  const turn = await session.waitForTurnEnd<Turn>(turnId);
  assert.equal(turn.state, 'complete', 'provider must continue and finish after owner tool completion');
  await observer.waitForState<ChatState>(session.chatUri, (chat) => chat.turns.some((entry) => entry.id === turnId));
  return { turn, result, echo };
}

export async function assertCallMatchesSnapshot(t: TestContext, client: Client, chat: string, turnId: string, toolCallId: string) {
  const reader = await t.client({ initialSubscriptions: [] });
  const snapshot = (await reader.subscribe(chat)).state as ChatState;
  assert.deepEqual(normalized(callIn(client.state<ChatState>(chat), turnId, toolCallId)), normalized(callIn(snapshot, turnId, toolCallId)), 'client tool state differs from fresh snapshot');
}
