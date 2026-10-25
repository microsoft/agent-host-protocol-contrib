import type { RootState, TerminalState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { catalogue, echo, rootChannel, state, terminalTest } from './helpers.ts';

export const tests: TestDefinition[] = [
  terminalTest('createTerminal', 'create-initial-state', 'createTerminal creates valid subscribable state with its requested owner and initial dimensions.', async (_t, terminal) => {
    const { snapshot, initial } = terminal;
    assert.equal(typeof snapshot.title, 'string');
    assert.deepEqual(snapshot.claim, initial.claim);
    if (snapshot.cwd !== undefined) {
      assert.match(snapshot.cwd, /^[a-zA-Z][a-zA-Z0-9+.-]*:/, 'TerminalState.cwd must be a URI, not a filesystem path.');
    }
    if (snapshot.cols !== undefined) assert.equal(snapshot.cols, initial.cols);
    if (snapshot.rows !== undefined) assert.equal(snapshot.rows, initial.rows);
    assert.ok(Array.isArray(snapshot.content));
    assert.ok(snapshot.lifecycle.status === 'running' || snapshot.lifecycle.status === 'exited');
  }, '/reference/terminal#createterminal'),
  terminalTest('createTerminal', 'create-root-catalogue', 'createTerminal publishes matching TerminalInfo in the root catalogue.', async (_t, terminal) => {
    const { client, channel, snapshot } = terminal;
    const event = await client.waitForAction(rootChannel, (event) => event.action.type === 'root/terminalsChanged'
      && event.action.terminals.some((entry) => entry.resource === channel));
    assert.equal(event.channel, rootChannel);
    const info = await catalogue(client, channel);
    assert.equal(info.title, snapshot.title);
    assert.deepEqual(info.claim, snapshot.claim);
    const current = await state(client, channel);
    assert.deepEqual(info.lifecycle, current.lifecycle);
  }, '/specification/terminal-channel#creation'),
  terminalTest('disposeTerminal', 'dispose-removes-catalogue', 'disposeTerminal removes a running terminal from the root catalogue.', async (_t, terminal) => {
    const { client, channel } = terminal;
    await catalogue(client, channel);
    const after = client.lastServerSeq;
    await terminal.dispose();
    await client.waitForAction(rootChannel, (event) => event.serverSeq > after
      && event.action.type === 'root/terminalsChanged' && !event.action.terminals.some((entry) => entry.resource === channel));
    assert.ok(!client.state<RootState>(rootChannel)?.terminals?.some((entry) => entry.resource === channel));
  }, '/specification/terminal-channel#disposal'),
  terminalTest('terminal/exited', 'exit-lifecycle-catalogue', 'A terminal exit is reflected in terminal state and its root catalogue lifecycle.', async (t, terminal) => {
    await echo(t, terminal);
    const { client, channel } = terminal;
    const after = client.lastServerSeq;
    client.dispatch(channel, { type: 'terminal/input', data: 'exit 7\r' });
    const exited = await client.waitForAction(channel, (event) => event.serverSeq > after && event.action.type === 'terminal/exited');
    assert.equal(exited.action.type, 'terminal/exited');
    const current = await state(client, channel);
    assert.equal(current.lifecycle.status, 'exited');
    if (current.lifecycle.status === 'exited' && exited.action.type === 'terminal/exited') {
      assert.equal(current.lifecycle.exitCode, exited.action.exitCode);
      if (current.lifecycle.exitCode !== undefined) assert.equal(current.lifecycle.exitCode, 7);
    }
    const info = await catalogue(client, channel, (entry) => entry.lifecycle.status === 'exited');
    assert.deepEqual(info.lifecycle, current.lifecycle as TerminalState['lifecycle']);
  }, '/specification/terminal-channel#control'),
];
