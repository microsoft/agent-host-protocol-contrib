import { assert } from '../../harness/assert.ts';
import { prompts } from '../../harness/prompts.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, TestContext, TestDefinition } from '../../harness/types.ts';
import {
  accepted, after, createChat, dispatch, headSpec, record, sessionSetup, snapshot, startTurn,
  turnEnded, userMessage,
} from './helpers.ts';
import type { WireEnvelope } from './helpers.ts';

const spec = (section: string) => headSpec('docs/specification/chat-channel.md', section);

function entries(state: Record<string, unknown>) {
  return Array.isArray(state.queuedMessages) ? state.queuedMessages.map(record) : [];
}

async function pendingSetup(t: TestContext) {
  const setup = await sessionSetup(t);
  const { owner, session } = setup;
  const id = t.uniqueId('pending-support');
  const cursor = owner.received.length;
  const clientSeq = dispatch(owner, session.chatUri, {
    type: 'chat/pendingMessageSet', id, kind: 'steering', message: userMessage(t.uniqueId('steering-probe')),
  });
  try {
    await owner.waitForMessage(after(owner, cursor, (message) => {
      if (message.method !== 'action') return false;
      const envelope = message.params as WireEnvelope;
      return envelope.channel === session.chatUri && (envelope.origin?.clientSeq === clientSeq
        && envelope.origin.clientId === owner.clientId || envelope.action.queuedMessageId === id);
    }), { timeoutMs: 2000 });
  } catch (error) {
    if (t.signal.aborted || !(error instanceof Error) || !/timed out/i.test(error.message)) throw error;
    t.skip('Host did not acknowledge the shared-pending-list support probe within 2 seconds');
  }
  const state = (await snapshot(owner, session.chatUri)).state;
  const probe = entries(state).find((entry) => entry.id === id && entry.kind === 'steering');
  if (!probe) t.skip('Host did not expose a retained kind-bearing steering entry in queuedMessages; shared-list support is unestablished (legacy singleton or immediate handoff)');
  return { ...setup, probeId: id };
}

async function busyPendingSetup(t: TestContext) {
  const setup = await pendingSetup(t);
  const { owner, session, probeId } = setup;
  await accepted(owner, session.chatUri, { type: 'chat/pendingMessageRemoved', id: probeId, kind: 'steering' });
  const text = Array.from({ length: 128 }, (_, index) => `${t.uniqueId('busy')}-${index}`).join(' ');
  const turnId = startTurn(t, owner, session.chatUri, prompts.reply(text, { chunks: 32, waitMs: 100 }));
  await owner.waitForMessage((message) => message.method === 'action'
    && (message.params as WireEnvelope).channel === session.chatUri
    && (message.params as WireEnvelope).action.type === 'chat/turnStarted'
    && (message.params as WireEnvelope).action.turnId === turnId);
  if (record((await snapshot(owner, session.chatUri)).state.activeTurn ?? {}).id !== turnId) {
    t.skip('Setup turn completed before an editable mixed pending-list interval was available');
  }
  return { ...setup, turnId };
}

function assertStillPending(t: TestContext, client: Client, chat: string, cursor: number,
  state: Record<string, unknown>, ids: string[]) {
  const list = entries(state);
  const missing = ids.filter((id) => !list.some((entry) => entry.id === id));
  const consumed = client.received.slice(cursor).some((message) => {
    if (message.method !== 'action') return false;
    const envelope = message.params as WireEnvelope;
    return envelope.channel === chat && !envelope.rejectionReason && (
      envelope.action.type === 'chat/pendingMessageRemoved' && missing.includes(String(envelope.action.id))
      || envelope.action.type === 'chat/turnStarted' && missing.includes(String(envelope.action.queuedMessageId)));
  });
  if (missing.length && consumed) t.skip('Host legitimately handed pending input to its provider before the raw snapshot could observe the test mutation');
  assert.deepEqual(missing, [], 'accepted pending mutation lost entries without reported consumption');
  return list;
}

function chatConfig(state: Record<string, unknown>): Record<string, unknown> | undefined {
  return state.config === undefined ? undefined : record(record(state.config).values);
}

function mutableChoice(t: TestContext, state: Record<string, unknown>, chat: Record<string, unknown>) {
  if (!state.config) t.skip('Proposed session exposes no config schema');
  const config = record(state.config);
  const properties = record(record(config.schema).properties);
  const values = record(config.values);
  for (const [key, value] of Object.entries(properties)) {
    const schema = record(value);
    if (schema.chatMutable !== true || schema.readOnly || schema.enumDynamic) continue;
    const current = chatConfig(chat)?.[key] ?? values[key];
    const choices = Array.isArray(schema.enum) ? schema.enum : schema.type === 'boolean' ? [false, true] : [];
    const alternate = choices.find((choice) => JSON.stringify(choice) !== JSON.stringify(current));
    if (alternate !== undefined) return { key, value: alternate, values };
  }
  t.skip('No advertised chatMutable boolean/enumerated property has a safe alternative value');
}

export const tests: TestDefinition[] = [
  {
    id: 'proposed/pending/distinct-steering-ids-survive', area: 'Proposed', feature: 'pendingMessageSet',
    title: 'Distinct steering submissions survive in the shared pending list in acceptance order.',
    spec: spec('shared-list-mutations-and-ordering'), requires: [requires.proposed],
    async run(t) {
      const { owner, session, probeId } = await pendingSetup(t);
      const id = t.uniqueId('second-steering');
      const message = userMessage(t.uniqueId('second-steering-text'));
      await accepted(owner, session.chatUri, { type: 'chat/pendingMessageSet', id, kind: 'steering', message });
      const state = (await snapshot(owner, session.chatUri)).state;
      const list = entries(state);
      assert.deepEqual(list.map((entry) => entry.id), [probeId, id]);
      assert.deepEqual(list.map((entry) => entry.kind), ['steering', 'steering']);
      assert.deepEqual(list[1]!.message, message);
      assert.equal(record(state.steeringMessage).id, id);
    },
  },
  {
    id: 'proposed/pending/reorder-mixed-list', area: 'Proposed', feature: 'queuedMessagesReordered',
    title: 'Mixed-list reordering ignores unknown and duplicate IDs without grouping by kind.',
    spec: spec('shared-list-mutations-and-ordering'), requires: [requires.proposed, requires.model],
    async run(t) {
      const { owner, session } = await busyPendingSetup(t);
      const ids = ['queued', 'steering', 'queued', 'steering'].map((kind) => t.uniqueId(kind));
      const cursor = owner.received.length;
      for (let index = 0; index < ids.length; index++) await accepted(owner, session.chatUri, {
        type: 'chat/pendingMessageSet', id: ids[index], kind: index % 2 ? 'steering' : 'queued', message: userMessage(ids[index]!),
      });
      assertStillPending(t, owner, session.chatUri, cursor, (await snapshot(owner, session.chatUri)).state, ids);
      const reorderCursor = owner.received.length;
      await accepted(owner, session.chatUri, {
        type: 'chat/queuedMessagesReordered', order: [ids[2], ids[1], ids[2], t.uniqueId('unknown')],
      });
      const list = assertStillPending(t, owner, session.chatUri, reorderCursor, (await snapshot(owner, session.chatUri)).state, ids);
      assert.deepEqual(list.map((entry) => entry.id), [ids[2], ids[1], ids[0], ids[3]]);
      assert.deepEqual(list.map((entry) => entry.kind), ['queued', 'steering', 'queued', 'steering']);
    },
  },
  {
    id: 'proposed/pending/kind-change-is-id-upsert', area: 'Proposed', feature: 'pendingMessageSet',
    title: 'Changing an existing pending ID’s kind replaces it in place and clears its legacy steering mirror.',
    spec: spec('legacy-steering-compatibility'), requires: [requires.proposed, requires.model],
    async run(t) {
      const { owner, session } = await busyPendingSetup(t);
      const ids = [t.uniqueId('before'), t.uniqueId('changing'), t.uniqueId('after')];
      const cursor = owner.received.length;
      for (let index = 0; index < ids.length; index++) await accepted(owner, session.chatUri, {
        type: 'chat/pendingMessageSet', id: ids[index], kind: index === 1 ? 'steering' : 'queued', message: userMessage(ids[index]!),
      });
      assertStillPending(t, owner, session.chatUri, cursor, (await snapshot(owner, session.chatUri)).state, ids);
      const changed = userMessage(t.uniqueId('updated-message'));
      const changeCursor = owner.received.length;
      const echo = await accepted(owner, session.chatUri, { type: 'chat/pendingMessageSet', id: ids[1], kind: 'queued', message: changed }, true);
      if (echo.rejectionReason && /handoff|processing|consum|deliver|locked/i.test(echo.rejectionReason)) {
        t.skip(`Host handed pending input to its provider before the kind edit: ${echo.rejectionReason}`);
      }
      assert.equal(echo.rejectionReason, undefined, echo.rejectionReason ?? 'kind change was rejected');
      const state = (await snapshot(owner, session.chatUri)).state;
      const list = assertStillPending(t, owner, session.chatUri, changeCursor, state, ids);
      assert.deepEqual(list.map((entry) => entry.id), ids);
      assert.equal(list[1]!.kind, 'queued');
      assert.deepEqual(list[1]!.message, changed);
      assert.equal(state.steeringMessage, undefined);
    },
  },
  {
    id: 'proposed/chat-config/sibling-and-session-isolation', area: 'Proposed', feature: 'chat/configChanged',
    title: 'An advertised chat-mutable value affects only the selected chat’s overlay.',
    spec: spec('per-chat-configuration'), requires: [requires.proposed, requires.multipleChats],
    async run(t) {
      const { owner, session } = await sessionSetup(t);
      const beforeSession = (await snapshot(owner, session.sessionUri)).state;
      const beforeChat = (await snapshot(owner, session.chatUri)).state;
      const choice = mutableChoice(t, beforeSession, beforeChat);
      const sibling = await createChat(t, owner, session.sessionUri);
      const beforeSibling = chatConfig((await snapshot(owner, sibling)).state);
      const beforeChangeSessionValues = record((await snapshot(owner, session.sessionUri)).state.config).values;
      await accepted(owner, session.chatUri, { type: 'chat/configChanged', config: { [choice.key]: choice.value } });
      assert.deepEqual(chatConfig((await snapshot(owner, session.chatUri)).state)?.[choice.key], choice.value);
      assert.deepEqual(record((await snapshot(owner, session.sessionUri)).state.config).values, beforeChangeSessionValues);
      assert.deepEqual(chatConfig((await snapshot(owner, sibling)).state), beforeSibling);
    },
  },
  {
    id: 'proposed/chat-config/replace-resets-to-session-value', area: 'Proposed', feature: 'chat/configChanged',
    title: 'Replacing a chat overlay without a key resets that property to inherited session configuration.',
    spec: spec('per-chat-configuration'), requires: [requires.proposed],
    async run(t) {
      const { owner, session } = await sessionSetup(t);
      const beforeSession = (await snapshot(owner, session.sessionUri)).state;
      const choice = mutableChoice(t, beforeSession, (await snapshot(owner, session.chatUri)).state);
      await accepted(owner, session.chatUri, { type: 'chat/configChanged', config: { [choice.key]: choice.value } });
      assert.deepEqual(chatConfig((await snapshot(owner, session.chatUri)).state)?.[choice.key], choice.value);
      await accepted(owner, session.chatUri, { type: 'chat/configChanged', config: {}, replace: true });
      const values = chatConfig((await snapshot(owner, session.chatUri)).state) ?? {};
      assert.equal(Object.hasOwn(values, choice.key), false, 'reset copied an inherited value into the overlay');
      assert.deepEqual(record((await snapshot(owner, session.sessionUri)).state.config).values, choice.values);
    },
  },
  {
    id: 'proposed/fork/initial-message-after-source-prefix', area: 'Proposed', feature: 'createChat',
    title: 'A fork’s initial message starts one new turn after its copied source history.',
    spec: spec('creation'), requires: [requires.proposed, requires.fork, requires.model],
    async run(t) {
      const { owner, session } = await sessionSetup(t);
      const root = (await snapshot(owner, 'ahp-root://')).state;
      const agent = (root.agents as Record<string, unknown>[]).find((value) => value.provider === t.profile.agent.provider);
      const chats = agent?.capabilities && record(agent.capabilities).multipleChats;
      if (!chats || record(chats).fork !== true) t.skip('Selected agent does not advertise fork on the proposed connection');
      const sourceId = startTurn(t, owner, session.chatUri, prompts.reply(t.uniqueId('source-response')));
      await turnEnded(owner, session.chatUri, sourceId);
      const source = (await snapshot(owner, session.chatUri)).state.turns as Record<string, unknown>[];
      const prompt = prompts.reply(t.uniqueId('fork-response'));
      const initialMessage = { ...userMessage(prompt), ...(t.profile.model && { model: { id: t.profile.model.id } }) };
      const chat = await createChat(t, owner, session.sessionUri, {
        source: { kind: 'fork', chat: session.chatUri, turnId: sourceId }, initialMessage,
      });
      let state = (await snapshot(owner, chat)).state;
      while (!(state.activeTurn || (state.turns as unknown[]).length > source.length)) {
        const cursor = owner.received.length;
        await owner.waitForMessage(after(owner, cursor, (message) => message.method === 'action' && (message.params as WireEnvelope).channel === chat));
        state = (await snapshot(owner, chat)).state;
      }
      const turns = state.turns as Record<string, unknown>[];
      assert.deepEqual(turns.slice(0, source.length).map((turn) => turn.message), source.map((turn) => turn.message));
      const visible = [...turns, ...(state.activeTurn ? [record(state.activeTurn)] : [])];
      assert.equal(visible.length, source.length + 1);
      assert.equal(record(visible[source.length]!.message).text, prompt);
      assert.equal(record(record(visible[source.length]!.message).origin).kind, 'user');
    },
  },
  {
    id: 'proposed/chat/initial-title', area: 'Proposed', feature: 'createChat',
    title: 'A host handling initial titles publishes the client-selected title on the new chat.',
    spec: spec('creation'), requires: [requires.proposed, requires.multipleChats],
    async run(t) {
      const { owner, session } = await sessionSetup(t);
      const probeTitle = t.uniqueId('title-support');
      const probe = await createChat(t, owner, session.sessionUri, { title: probeTitle });
      if ((await snapshot(owner, probe)).state.title !== probeTitle) {
        t.skip('Host ignored CreateChatParams.title; HEAD has no initialTitle capability and 1.1.0 negotiation alone does not establish support');
      }
      const title = t.uniqueId('selected-title');
      const cursor = owner.received.length;
      const chat = await createChat(t, owner, session.sessionUri, { title });
      const added = owner.received.slice(cursor).find((message) => {
        if (message.method !== 'action') return false;
        const envelope = message.params as WireEnvelope;
        return envelope.channel === session.sessionUri && envelope.action.type === 'session/chatAdded'
          && record(envelope.action.summary).resource === chat;
      });
      assert.ok(added, 'missing initial session/chatAdded summary');
      assert.equal(record((added.params as WireEnvelope).action.summary).title, title);
      assert.equal((await snapshot(owner, chat)).state.title, title);
    },
  },
];
