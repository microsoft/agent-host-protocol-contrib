import { stripVTControlCharacters } from 'node:util';
import { AhpErrorCodes, JsonRpcErrorCodes } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, RootState, TerminalClaim, TerminalInfo, TerminalState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, LooseStateAction, TestContext, TestDefinition } from '../../harness/types.ts';

export const rootChannel = 'ahp-root://';

export function clientClaim(clientId: string): TerminalClaim {
  return { kind: 'client', clientId } as unknown as TerminalClaim;
}

export async function dispose(client: Client, channel: string) {
  try {
    await client.request('disposeTerminal', { channel });
  } catch (error) {
    if (!(error instanceof RpcError && error.code === AhpErrorCodes.NotFound)) throw error;
  }
}

export async function state(client: Client, channel: string): Promise<TerminalState> {
  const snapshot = await client.subscribe(channel);
  assert.equal(snapshot.resource, channel);
  return snapshot.state as TerminalState;
}

export async function terminal(t: TestContext) {
  const client = await t.client();
  const channel = `ahp-terminal:/${t.uniqueId('terminal')}`;
  const initial = { name: t.uniqueId('terminal-title'), cwd: t.scratchUri(), cols: 120, rows: 30,
    claim: clientClaim(client.clientId) };
  try {
    await client.request('createTerminal', { channel, ...initial });
  } catch (error) {
    if (error instanceof RpcError && error.code === JsonRpcErrorCodes.MethodNotFound) {
      t.skip('Host does not support terminals (createTerminal returned MethodNotFound)');
    }
    throw error;
  }
  let disposed = false;
  const close = async () => {
    if (disposed) return;
    await dispose(client, channel);
    disposed = true;
  };
  t.defer(close);
  const snapshot = await state(client, channel);
  return { client, channel, initial, snapshot, dispose: close };
}

export type Terminal = Awaited<ReturnType<typeof terminal>>;

export function terminalTest(
  feature: string,
  name: string,
  title: string,
  run: (t: TestContext, terminal: Terminal) => Promise<void>,
  spec = '/specification/terminal-channel#control',
  requirements: TestDefinition['requires'] = [],
): TestDefinition {
  const slug = feature.replace(/^terminal\//, '').replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
  return { id: `terminals/${slug}/${name}`, area: 'Terminals', feature, title, spec, requires: requirements,
    async run(t) { await run(t, await terminal(t)); } };
}

export async function accepted(client: Client, channel: string, action: LooseStateAction) {
  const clientSeq = client.dispatch(channel, action);
  const envelope = await client.waitForAction(channel, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
  assert.equal(envelope.rejectionReason, undefined, envelope.rejectionReason ?? 'terminal action rejected');
  return envelope;
}

export async function catalogue(client: Client, channel: string, predicate: (info: TerminalInfo) => boolean = () => true) {
  const root = await client.waitForState<RootState>(rootChannel, (root) =>
    root.terminals?.some((entry) => entry.resource === channel && predicate(entry)) === true);
  return root.terminals!.find((entry) => entry.resource === channel)!;
}

export function contentText(value: TerminalState): string {
  return value.content.map((part) => part.type === 'command' ? part.output : part.value).join('');
}

export function marker(t: TestContext): string {
  return t.uniqueId('terminal-output').replace(/[^a-zA-Z0-9-]/g, '-');
}

export async function echo(t: TestContext, terminal: Terminal) {
  const token = marker(t);
  const after = terminal.client.lastServerSeq;
  terminal.client.dispatch(terminal.channel, { type: 'terminal/input', data: `echo ${token}\r` });
  await terminal.client.waitForAction(terminal.channel, (event) => {
    if (event.serverSeq <= after || event.action.type !== 'terminal/data') return false;
    const output = terminal.client.actions(terminal.channel).flatMap((entry) =>
      entry.serverSeq > after && entry.action.type === 'terminal/data' ? [entry.action.data] : []).join('');
    const withoutOsc = output.replace(/(?:\x1b\]|\x9d)[\s\S]*?(?:\x07|\x1b\\|\x9c)/g, '');
    return stripVTControlCharacters(withoutOsc).split(/\r?\n/).some((line) => line.trim() === token);
  });
  return token;
}

export async function quiescent(t: TestContext, terminal: Terminal) {
  for (let attempt = 0; attempt < 5; attempt++) {
    const after = terminal.client.lastServerSeq;
    const isNewData = (event: ActionEnvelope) => event.channel === terminal.channel && event.serverSeq > after
      && event.action.type === 'terminal/data';
    try {
      await terminal.client.expectNoNotification<ActionEnvelope>('action', isNewData, 350);
      return;
    } catch (error) {
      if (!terminal.client.actions(terminal.channel).some(isNewData)) throw error;
    }
  }
  t.skip('The terminal kept emitting output before the clear-content precondition could be established.');
}
