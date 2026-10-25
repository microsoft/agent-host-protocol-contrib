import { stat, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import type {
  ActionEnvelope,
  ChatState,
  ToolCallPendingConfirmationState,
  ToolCallPendingResultConfirmationState,
  ToolCallState,
  Turn,
} from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, LooseStateAction, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';

type Decision = 'approve' | 'deny' | 'cancel';

export interface ToolTurnOptions {
  confirmation?(call: ToolCallPendingConfirmationState): Decision | Promise<Decision>;
  resultConfirmation?(call: ToolCallPendingResultConfirmationState): Decision | Promise<Decision>;
}

export interface ToolDecision {
  toolCall: ToolCallPendingConfirmationState | ToolCallPendingResultConfirmationState;
  decision: Decision;
}

export function toolTest(definition: Omit<TestDefinition, 'area' | 'requires'>): TestDefinition {
  return { area: 'Tools & permissions', requires: [requires.model, requires.sharedFilesystem], ...definition };
}

export function toolCalls(turn: Pick<Turn, 'responseParts'> | undefined): ToolCallState[] {
  return turn?.responseParts.flatMap((part) => part.kind === 'toolCall' ? [part.toolCall] : []) ?? [];
}

export async function dispatchAndWait(client: Client, channel: string, action: LooseStateAction): Promise<ActionEnvelope> {
  const clientSeq = client.dispatch(channel, action);
  return client.waitForAction(channel, (envelope) =>
    envelope.origin?.clientId === client.clientId && envelope.origin.clientSeq === clientSeq);
}

export async function dispatchAccepted(client: Client, channel: string, action: LooseStateAction): Promise<ActionEnvelope> {
  const envelope = await dispatchAndWait(client, channel, action);
  assert.equal(envelope.rejectionReason, undefined, `action rejected: ${envelope.rejectionReason}`);
  return envelope;
}

export async function runToolTurn(client: Client, session: SessionHandle, prompt: string, options: ToolTurnOptions = {}) {
  const startedAt = performance.now();
  const turnId = session.startTurn(`If a tool invocation or result is denied, stop without retrying or using another tool to perform the operation.\n\n${prompt}`);
  const decisions: ToolDecision[] = [];
  while (true) {
    const state = await client.waitForState<ChatState>(session.chatUri, (state) =>
      state.turns.some((turn) => turn.id === turnId) ||
      (state.activeTurn?.id === turnId && toolCalls(state.activeTurn).some((call) =>
        call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation')));
    const turn = state.turns.find((turn) => turn.id === turnId);
    if (turn) return { turnId, turn, decisions };
    const pending = toolCalls(state.activeTurn).find((call) =>
      call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation');
    assert(pending);
    if (pending.status !== 'pending-confirmation' && pending.status !== 'pending-result-confirmation') continue;
    const decision = pending.status === 'pending-confirmation'
      ? await options.confirmation?.(pending) ?? 'approve'
      : await options.resultConfirmation?.(pending) ?? 'approve';
    decisions.push({ toolCall: pending, decision });
    if (decision === 'cancel') {
      await dispatchAccepted(client, session.chatUri, {
        type: 'chat/turnCancelled', turnId, duration: Math.max(0, performance.now() - startedAt),
      } as LooseStateAction);
      return { turnId, turn: await session.waitForTurnEnd<Turn>(turnId), decisions };
    }
    if (pending.status === 'pending-result-confirmation') {
      await dispatchAccepted(client, session.chatUri, {
        type: 'chat/toolCallResultConfirmed', turnId, toolCallId: pending.toolCallId, approved: decision === 'approve',
      } as LooseStateAction);
    } else {
      const selectedOptionId = pending.options?.find((option) => option.kind === decision)?.id;
      await dispatchAccepted(client, session.chatUri, decision === 'approve' ? {
        type: 'chat/toolCallConfirmed', turnId, toolCallId: pending.toolCallId,
        approved: true, confirmed: 'user-action', selectedOptionId,
      } as LooseStateAction : {
        type: 'chat/toolCallConfirmed', turnId, toolCallId: pending.toolCallId,
        approved: false, reason: 'denied', selectedOptionId,
      } as LooseStateAction);
    }
  }
}

export async function readTurn(t: TestContext, options: ToolTurnOptions = {}) {
  const token = t.uniqueId('file-token');
  const filePath = join(t.scratchDir, `${t.uniqueId('read')}.txt`);
  await writeFile(filePath, `${token}\n`, { flag: 'wx' });
  const client = await t.client();
  const session = await t.session(client);
  return { client, session, token, filePath, ...await runToolTurn(client, session, prompts.readFile(filePath), options) };
}

export function writeTarget(t: TestContext) {
  const filePath = join(t.scratchDir, `${t.uniqueId('write')}.txt`);
  const contents = `${t.uniqueId('written-token')}\nsecond line\n`;
  return { filePath, contents, prompt: prompts.writeFile(filePath, contents) };
}

export async function fileExists(filePath: string): Promise<boolean> {
  try {
    await stat(filePath);
    return true;
  } catch (error) {
    if (error instanceof Error && 'code' in error && error.code === 'ENOENT') return false;
    throw error;
  }
}

export async function writeTurn(t: TestContext, options: ToolTurnOptions = {}) {
  const target = writeTarget(t);
  const client = await t.client();
  const session = await t.session(client);
  return { client, session, ...target, ...await runToolTurn(client, session, target.prompt, options) };
}

export function requireConfirmation(t: TestContext, decisions: ToolDecision[]) {
  const confirmations = decisions.filter((decision) => decision.toolCall.status === 'pending-confirmation');
  if (!confirmations.length) t.skip('host did not request confirmation');
  return confirmations;
}

export async function cancelledWriteTurn(t: TestContext) {
  const result = await writeTurn(t, { confirmation: () => 'cancel' });
  requireConfirmation(t, result.decisions);
  return result;
}
