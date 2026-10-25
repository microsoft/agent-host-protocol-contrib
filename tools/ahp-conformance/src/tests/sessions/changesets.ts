import { readFile } from 'node:fs/promises';
import { isDeepStrictEqual } from 'node:util';
import { changesetReducer } from '@microsoft/agent-host-protocol';
import type { Changeset, ChangesetState, ChatState, SessionState, Snapshot } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, SessionHandle, TestContext, TestDefinition } from '../../harness/types.ts';
import { writeTarget, writeTurn } from './coverage-helpers.ts';

function expand(template: string, chat: ChatState): string | undefined {
  const variables = [...template.matchAll(/\{([^{}]+)\}/g)].map((match) => match[1]);
  if (!variables.length) return template;
  if (variables.some((name) => !['turnId', 'originalTurnId', 'modifiedTurnId'].includes(name))) return undefined;
  const turns = chat.turns;
  if (variables.includes('turnId') && !turns.length) return undefined;
  if (variables.some((name) => name !== 'turnId') &&
      (!variables.includes('originalTurnId') || !variables.includes('modifiedTurnId') || turns.length < 2)) return undefined;
  const values: Record<string, string | undefined> = {
    turnId: turns.at(-1)?.id, originalTurnId: turns.at(-2)?.id, modifiedTurnId: turns.at(-1)?.id,
  };
  return template.replace(/\{([^{}]+)\}/g, (_, name: string) => encodeURIComponent(values[name]!));
}

async function advertisedChangeset(t: TestContext, client: Client, session: SessionHandle, live = false) {
  const state = (await client.subscribe(session.sessionUri)).state as SessionState;
  const candidates: { entry: Changeset; chat: string; state: ChatState }[] = [];
  const primary = client.state<ChatState>(session.chatUri);
  assert.ok(primary, 'the initial chat must be subscribed');
  for (const entry of state.changesets ?? []) candidates.push({ entry, chat: session.chatUri, state: primary });
  for (const chat of state.chats) {
    const snapshot = (await client.subscribe(chat.resource)).state as ChatState;
    for (const entry of snapshot.changesets ?? []) candidates.push({ entry, chat: chat.resource, state: snapshot });
  }
  for (const candidate of candidates) {
    if (live && (!['session', 'uncommitted'].includes(candidate.entry.changeKind) || /[{}]/.test(candidate.entry.uriTemplate))) continue;
    const uri = expand(candidate.entry.uriTemplate, candidate.state);
    if (uri) return { uri, chat: candidate.chat };
  }
  t.skip(live
    ? 'No advertised static session/uncommitted changeset can observe a file write in the scratch workspace.'
    : 'No advertised session/chat changeset has a URI template satisfiable from retained turns.');
}

function reduce(client: Client, snapshot: Snapshot, through = Infinity): ChangesetState {
  let state = snapshot.state as ChangesetState;
  for (const event of client.actions(snapshot.resource)) {
    if (event.serverSeq <= snapshot.fromSeq || event.serverSeq > through || event.rejectionReason !== undefined) continue;
    if (!event.action.type.startsWith('changeset/')) continue;
    state = changesetReducer(state, event.action as Parameters<typeof changesetReducer>[1]);
  }
  return state;
}

async function waitForChangeset(client: Client, snapshot: Snapshot, predicate: (state: ChangesetState) => boolean) {
  if (!predicate(reduce(client, snapshot))) {
    await client.waitForMessage((message) => message.method === 'action' && predicate(reduce(client, snapshot)), { timeoutMs: 5_000 });
  }
  return reduce(client, snapshot);
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/changesets/unknown-operation-rejected',
    area: 'Root & sessions', feature: 'invokeChangesetOperation',
    title: 'Invoking an operation absent from an advertised changeset returns a JSON-RPC error.',
    spec: '/reference/changeset#invokechangesetoperation',
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const changeset = await advertisedChangeset(t, client, session);
      const snapshot = await client.subscribe(changeset.uri);
      const state = snapshot.state as ChangesetState;
      const operationId = t.uniqueId('missing-operation');
      assert.ok(!state.operations?.some((operation) => operation.id === operationId), 'generated operation id must be absent');
      await assert.rejects(client.request('invokeChangesetOperation', { channel: changeset.uri, operationId }), RpcError);
    },
  },
  {
    id: 'sessions/changesets/live-mirror-matches-snapshot',
    area: 'Root & sessions', feature: 'changesets',
    title: 'Live changeset actions converge to a fresh snapshot after a scratch-workspace file write.',
    spec: '/guide/changesets#changeset-state', requires: [requires.model, requires.sharedFilesystem],
    async run(t) {
      const client = await t.client();
      const session = await t.session(client);
      await session.ready();
      const changeset = await advertisedChangeset(t, client, session, true);
      const initial = await client.subscribe(changeset.uri);
      const ready = await waitForChangeset(client, initial, (state) => state.status === 'ready' || state.status === 'error');
      if (ready.status === 'error') t.skip('The advertised changeset computation failed before the live-update setup.');
      const baseline = await client.subscribe(changeset.uri);
      const target = writeTarget(t);
      await writeTurn(t, client, session, target.prompt, changeset.chat);
      let written: string;
      try {
        written = await readFile(target.path, 'utf8');
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === 'ENOENT') t.skip('The prerequisite model turn did not create the scratch file.');
        throw error;
      }
      if (written !== target.contents) t.skip('The prerequisite model turn did not write the requested scratch contents.');
      try {
        await client.waitForAction(changeset.uri, (event) => event.serverSeq > baseline.fromSeq &&
          event.rejectionReason === undefined && event.action.type.startsWith('changeset/'), { timeoutMs: 5_000 });
      } catch (error) {
        t.signal.throwIfAborted();
        if (!(error instanceof Error) || error.message !== 'Waiting for message timed out after 5000 ms') throw error;
      }
      const observer = await t.client({ initialSubscriptions: [] });
      const fresh = await observer.subscribe(changeset.uri);
      await client.request('ping', { channel: 'ahp-root://' });
      const normalize = (value: unknown) => JSON.parse(JSON.stringify(value));
      const events = client.actions(changeset.uri).filter((event) => event.serverSeq > baseline.fromSeq &&
        event.serverSeq <= fresh.fromSeq && event.rejectionReason === undefined && event.action.type.startsWith('changeset/'));
      if (!events.length && isDeepStrictEqual(normalize(baseline.state), normalize(fresh.state))) {
        t.skip('The advertised view had no observable state mutation after the scratch write; it may exclude untracked files.');
      }
      assert.deepEqual(normalize(reduce(client, baseline, fresh.fromSeq)), normalize(fresh.state));
    },
  },
];
