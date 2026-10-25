import { rm } from 'node:fs/promises';
import { join } from 'node:path';
import type {
  ActionEnvelope, ChatState, SessionSummaryChangedParams,
  ToolCallPendingConfirmationState, ToolCallPendingResultConfirmationState, ToolCallState, Turn,
} from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, LooseStateAction, SessionHandle, TestContext } from '../../harness/types.ts';

export const INPUT_NEEDED = (1 << 3) | (1 << 4);
export const ERROR = 1 << 1;

export function jsonValue(value: unknown): unknown {
  return value === undefined ? undefined : JSON.parse(JSON.stringify(value));
}

export async function accepted(client: Client, channel: string, action: LooseStateAction, prerequisite?: TestContext) {
  const seq = client.dispatch(channel, action);
  const echo = await client.waitForAction(channel, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === seq);
  if (echo.rejectionReason && prerequisite) prerequisite.skip(`Setup action ${action.type} was rejected: ${echo.rejectionReason}`);
  assert.equal(echo.rejectionReason, undefined, `action rejected: ${echo.rejectionReason}`);
  return echo;
}

export async function additionalChat(t: TestContext, client: Client, session: SessionHandle) {
  const chat = `ahp-chat:/${t.uniqueId('chat')}`;
  try {
    await client.request('createChat', { channel: session.sessionUri, chat });
  } catch (error) {
    if (error instanceof RpcError) t.skip(`Additional-chat setup was rejected: ${error.message}`);
    throw error;
  }
  await client.subscribe(chat);
  return chat;
}

export function calls(turn: Pick<Turn, 'responseParts'> | undefined): ToolCallState[] {
  return turn?.responseParts.flatMap((part) => part.kind === 'toolCall' ? [part.toolCall] : []) ?? [];
}

export type Confirmation = ToolCallPendingConfirmationState | ToolCallPendingResultConfirmationState;

export function confirmationAction(turnId: string, call: Confirmation, approved: boolean): LooseStateAction {
  if (call.status === 'pending-result-confirmation') {
    return { type: 'chat/toolCallResultConfirmed', turnId, toolCallId: call.toolCallId, approved } as LooseStateAction;
  }
  return approved ? {
    type: 'chat/toolCallConfirmed', turnId, toolCallId: call.toolCallId,
    approved: true, confirmed: 'user-action',
  } as LooseStateAction : {
    type: 'chat/toolCallConfirmed', turnId, toolCallId: call.toolCallId,
    approved: false, reason: 'denied',
  } as LooseStateAction;
}

export function writeTarget(t: TestContext) {
  const name = `${t.uniqueId('session-write')}.txt`;
  const path = join(t.scratchDir, name);
  const contents = `${t.uniqueId('contents')}\n`;
  t.defer(() => rm(path, { force: true }));
  return { path, uri: t.scratchUri(name), contents, prompt: prompts.writeFile(path, contents) };
}

export async function heldConfirmation(t: TestContext, client: Client, session: SessionHandle, chat = session.chatUri) {
  const target = writeTarget(t);
  const turnId = session.startTurn(
    `If a tool invocation or result is denied, stop without retrying or using another tool.\n\n${target.prompt}`,
    { chatUri: chat },
  );
  const state = await client.waitForState<ChatState>(chat, (value) =>
    value.turns.some((turn) => turn.id === turnId) ||
    (value.activeTurn?.id === turnId && calls(value.activeTurn).some((call) =>
      call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation')));
  const call = state.activeTurn?.id === turnId ? calls(state.activeTurn).find((call) =>
    call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation') : undefined;
  if (!call || (call.status !== 'pending-confirmation' && call.status !== 'pending-result-confirmation')) {
    t.skip('The model turn ended without an observable pending tool confirmation.');
  }
  return { turnId, call, chat };
}

export async function writeTurn(t: TestContext, client: Client, session: SessionHandle, prompt: string, chat = session.chatUri) {
  const turnId = session.startTurn(prompt, { chatUri: chat });
  for (let confirmations = 0; confirmations < 64; confirmations++) {
    t.signal.throwIfAborted();
    const state = await client.waitForState<ChatState>(chat, (value) =>
      value.turns.some((turn) => turn.id === turnId) ||
      (value.activeTurn?.id === turnId && calls(value.activeTurn).some((call) =>
        call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation')));
    const turn = state.turns.find((turn) => turn.id === turnId);
    if (turn) {
      if (turn.state !== 'complete') t.skip('The prerequisite file-writing turn did not complete successfully.');
      return turn;
    }
    for (const call of calls(state.activeTurn)) {
      if (call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation') {
        await accepted(client, chat, confirmationAction(turnId, call, true), t);
      }
    }
  }
  t.skip('The prerequisite write exceeded the bounded budget of 64 tool-confirmation rounds.');
}

export async function summaryUpdate(
  client: Client, session: string, since: number,
  predicate: (changes: SessionSummaryChangedParams['changes']) => boolean,
) {
  const message = await client.waitForMessage((message) => {
    if (client.received.indexOf(message) < since || message.method !== 'root/sessionSummaryChanged') return false;
    const event = message.params as SessionSummaryChangedParams;
    return event.session === session && predicate(event.changes);
  }, { timeoutMs: 5_000 });
  return message.params as SessionSummaryChangedParams;
}

export function successful(envelope: ActionEnvelope) {
  return envelope.rejectionReason === undefined;
}
