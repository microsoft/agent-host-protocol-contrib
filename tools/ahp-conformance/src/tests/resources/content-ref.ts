import type { ChatState, ContentRef, ResourceReadResult, Turn } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, LooseStateAction, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';
import { dispatchAccepted, toolCalls } from '../tools/helpers.ts';
import { decoded, fixtureFile, rootChannel } from './helpers.ts';

type Response = Pick<Turn, 'responseParts'>;

async function referenceStimulus(
  t: TestContext,
  client: Client,
  session: SessionHandle,
  prompt: string,
  collect: (turn: Response) => ContentRef[],
): Promise<ContentRef[]> {
  const turnId = session.startTurn(prompt);
  const deadline = performance.now() + Math.min(30_000, t.config.run.timeoutMs / 2);
  while (true) {
    const remaining = Math.max(1, Math.ceil(deadline - performance.now()));
    let chat: ChatState;
    try {
      chat = await client.waitForState<ChatState>(session.chatUri, (value) => {
        const current = value.activeTurn?.id === turnId ? value.activeTurn : value.turns.find((turn) => turn.id === turnId);
        return current !== undefined && (collect(current).length > 0 || value.turns.some((turn) => turn.id === turnId)
          || toolCalls(current).some((call) => call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation'));
      }, { timeoutMs: remaining });
    } catch (error) {
      if (!(error instanceof Error && error.message.startsWith(`Waiting for state ${session.chatUri} timed out after `))) throw error;
      const value = client.state<ChatState>(session.chatUri);
      const current = value?.activeTurn?.id === turnId ? value.activeTurn : value?.turns.find((turn) => turn.id === turnId);
      const references = current ? collect(current) : [];
      if (references.length) return references;
      return t.skip('The bounded model stimulus produced no host-published ContentRef before its observation window ended.');
    }
    const current = chat.activeTurn?.id === turnId ? chat.activeTurn : chat.turns.find((turn) => turn.id === turnId);
    assert.ok(current);
    const references = collect(current);
    if (references.length || chat.turns.some((turn) => turn.id === turnId)) return references;
    if (performance.now() >= deadline) return t.skip('The bounded model stimulus produced no host-published ContentRef.');
    const pending = toolCalls(current).find((call) => call.status === 'pending-confirmation' || call.status === 'pending-result-confirmation');
    assert.ok(pending);
    if (pending.status === 'pending-result-confirmation') {
      await dispatchAccepted(client, session.chatUri, {
        type: 'chat/toolCallResultConfirmed', turnId, toolCallId: pending.toolCallId, approved: true,
      } as LooseStateAction);
    } else if (pending.status === 'pending-confirmation') {
      await dispatchAccepted(client, session.chatUri, {
        type: 'chat/toolCallConfirmed', turnId, toolCallId: pending.toolCallId, approved: true, confirmed: 'user-action',
        selectedOptionId: pending.options?.find((option) => option.kind === 'approve')?.id,
      } as LooseStateAction);
    }
  }
}

async function readReferences(client: Client, references: ContentRef[]) {
  for (const reference of references) {
    assert.equal(typeof reference.uri, 'string');
    const result = await client.request<ResourceReadResult>('resourceRead', { channel: rootChannel, uri: reference.uri });
    decoded(result);
  }
}

export const tests: TestDefinition[] = [
  {
    id: 'resources/content-ref/tool-result-readable', area: 'Resources', feature: 'ContentRef',
    title: 'Host-published tool-result content references can be read with resourceRead.',
    spec: '/reference/chat#toolresultresourcecontent', requires: [requires.model, requires.sharedFilesystem],
    async run(t) {
      const file = await fixtureFile(t, `${t.uniqueId('content')}\n`.repeat(8));
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const references = await referenceStimulus(t, client, session, prompts.readFile(file.path), (turn) =>
        toolCalls(turn).flatMap((call) => 'content' in call
          ? (call.content ?? []).flatMap((content) => content.type === 'resource' ? [content] : []) : []));
      if (!references.length) t.skip('The host did not publish a tool-result ContentRef for the read stimulus.');
      await readReferences(client, references);
    },
  },
  {
    id: 'resources/content-ref/response-part-readable', area: 'Resources', feature: 'ContentRef',
    title: 'Host-published response-part content references can be read with resourceRead.',
    spec: '/reference/chat#resourceresponsepart', requires: [requires.model],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const references = await referenceStimulus(t, client, session, prompts.reply(`${t.uniqueId('content')} `.repeat(8)), (turn) =>
        turn.responseParts.flatMap((part) => part.kind === 'contentRef' ? [part] : []));
      if (!references.length) t.skip('The host did not publish a response-part ContentRef for the reply stimulus.');
      await readReferences(client, references);
    },
  },
];
