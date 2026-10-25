import type { ActionEnvelope } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { LooseStateAction, TestDefinition } from '../../harness/types.ts';
import { contentText, marker, rootChannel, state, terminalTest } from './helpers.ts';

const actions = ['data', 'cwdChanged', 'exited', 'commandDetectionAvailable', 'commandExecuted', 'commandFinished'] as const;

export const tests: TestDefinition[] = actions.map((name) => terminalTest('serverOnly',
  `reject-server-only-${name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)}`,
  `Client dispatch cannot publish the server-only terminal/${name} action.`,
  async (t, terminal) => {
    const { client, channel } = terminal;
    const token = marker(t);
    const action = {
      data: { type: 'terminal/data', data: token },
      cwdChanged: { type: 'terminal/cwdChanged', cwd: t.scratchUri(token) },
      exited: { type: 'terminal/exited', exitCode: 123 },
      commandDetectionAvailable: { type: 'terminal/commandDetectionAvailable' },
      commandExecuted: { type: 'terminal/commandExecuted', commandId: token, commandLine: `echo ${token}`, timestamp: 1 },
      commandFinished: { type: 'terminal/commandFinished', commandId: token, exitCode: 123, durationMs: 1 },
    }[name] as LooseStateAction;
    const after = client.lastServerSeq;
    const clientSeq = client.dispatch(channel, action);
    await client.request('ping', { channel: rootChannel });
    const forged = (event: ActionEnvelope) => event.channel === channel && event.serverSeq > after && !event.rejectionReason
      && event.action.type === action.type && ((event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq)
        || (name !== 'commandDetectionAvailable' && Object.entries(action).every(([key, value]) =>
          (event.action as unknown as Record<string, unknown>)[key] === value)));
    assert.ok(!client.actions(channel).some(forged), 'The forged server-only action was accepted.');
    await client.expectNoNotification<ActionEnvelope>('action', forged, 350);
    const current = await state(client, channel);
    if (name === 'data') assert.ok(!contentText(current).includes(token));
    if (name === 'cwdChanged') assert.notEqual(current.cwd, t.scratchUri(token));
    if (name === 'exited') assert.notDeepEqual(current.lifecycle, { status: 'exited', exitCode: 123 });
    if (name === 'commandExecuted') assert.ok(!current.content.some((part) => part.type === 'command' && part.commandId === token));
  }, '/specification/terminal-channel#actions',
));
