import type { ChatState, ResourceReadResult, RootState, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, LooseStateAction, SessionHandle, TestDefinition } from '../../harness/types.ts';
import { toolCalls } from '../tools/helpers.ts';
import { accepted, contentText, dispose, marker, rootChannel, state, terminalTest } from './helpers.ts';

async function finishTurn(client: Client, session: SessionHandle, text: string): Promise<Turn> {
  const turnId = session.startTurn(text);
  while (true) {
    const chat = await client.waitForState<ChatState>(session.chatUri, (value) => value.turns.some((turn) => turn.id === turnId)
      || (value.activeTurn?.id === turnId && toolCalls(value.activeTurn).some((call) =>
        call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation')));
    const completed = chat.turns.find((turn) => turn.id === turnId);
    if (completed) return completed;
    const pending = toolCalls(chat.activeTurn).find((call) => call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation');
    assert.ok(pending);
    if (pending.status === 'pending-result-confirmation') {
      await accepted(client, session.chatUri, { type: 'chat/toolCallResultConfirmed', turnId, toolCallId: pending.toolCallId, approved: true });
    } else if (pending.status === 'pending-confirmation') {
      await accepted(client, session.chatUri, { type: 'chat/toolCallConfirmed', turnId, toolCallId: pending.toolCallId,
        approved: true, confirmed: 'user-action', selectedOptionId: pending.options?.find((option) => option.kind === 'approve')?.id } as LooseStateAction);
    }
  }
}

export const tests: TestDefinition[] = [terminalTest('commandPrefix', 'advertised-prefix-executes',
  'An advertised terminal command prefix executes the remainder of a chat message as a command.',
  async (t, terminal) => {
    const { client } = terminal;
    const session = await t.session(client);
    await session.ready();
    const prefix = client.initializeResult.terminalCommandPrefix;
    assert.ok(prefix);
    const token = marker(t);
    const resources = new Set<string>();
    t.defer(async () => {
      for (const info of client.state<RootState>(rootChannel)?.terminals ?? []) {
        if (info.claim.kind === 'session' && info.claim.session === session.sessionUri) resources.add(info.resource);
      }
      for (const resource of resources) await dispose(client, resource);
    });
    // Keep the prefix first: the generic tool-turn helper prepends model instructions.
    const turn = await finishTurn(client, session, `${prefix}echo ${token}`);
    assert.equal(turn.state, 'complete');
    const output = turn.responseParts.flatMap((part) => part.kind === 'markdown' ? [part.content] : []);
    const references: { kind: 'terminal' | 'resource'; uri: string }[] = turn.responseParts.flatMap((part) =>
      part.kind === 'contentRef' ? [{ kind: 'resource' as const, uri: part.uri }] : []);
    for (const call of toolCalls(turn)) {
      if (call.status !== 'completed') continue;
      for (const content of call.content ?? []) {
        if (content.type === 'text') output.push(content.text);
        if (content.type === 'terminal') {
          resources.add(content.resource);
          output.push(content.result?.preview ?? '');
          references.push({ kind: 'terminal', uri: content.resource });
        }
        if (content.type === 'resource') references.push({ kind: 'resource', uri: content.uri });
      }
    }
    for (const reference of references) {
      if (output.join('\n').trim().includes(token)) break;
      if (reference.kind === 'terminal') {
        output.push(contentText(await state(client, reference.uri)));
      } else {
        const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: reference.uri });
        assert.ok(result.encoding === 'utf-8' || result.encoding === 'base64');
        output.push(Buffer.from(result.data, result.encoding === 'base64' ? 'base64' : 'utf8').toString('utf8'));
      }
    }
    assert.ok(output.join('\n').trim().includes(token), 'The prefixed command did not expose its marker output.');
  }, '/reference/common#initializeresult', [requires.terminalCommandPrefix, requires.model],
)];
