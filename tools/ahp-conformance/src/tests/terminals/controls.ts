import { assert } from '../../harness/assert.ts';
import type { ActionEnvelope } from '@microsoft/agent-host-protocol';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, catalogue, clientClaim, contentText, echo, quiescent, rootChannel, state, terminalTest } from './helpers.ts';

export const tests: TestDefinition[] = [
  terminalTest('terminal/input', 'input-reaches-pty', 'Client terminal input produces server-authoritative process output.', async (t, terminal) => {
    const token = await echo(t, terminal);
    assert.ok(contentText(await state(terminal.client, terminal.channel)).includes(token));
  }, '/specification/terminal-channel#data-flow'),
  terminalTest('terminal/resized', 'resize-updates-pty', 'A client resize updates the authoritative terminal dimensions.', async (_t, terminal) => {
    const { client, channel } = terminal;
    await accepted(client, channel, { type: 'terminal/resized', cols: 93, rows: 27 });
    const current = await state(client, channel);
    assert.equal(current.cols, 93);
    assert.equal(current.rows, 27);
  }),
  terminalTest('terminal/titleChanged', 'title-change-catalogue', 'A client terminal rename is reflected in terminal state and root metadata.', async (t, terminal) => {
    const title = t.uniqueId('renamed');
    const { client, channel } = terminal;
    await accepted(client, channel, { type: 'terminal/titleChanged', title });
    assert.equal((await state(client, channel)).title, title);
    const info = await catalogue(client, channel, (entry) => entry.title === title);
    assert.equal(info.title, title);
  }),
  terminalTest('terminal/cleared', 'clear-content', 'A client clear action discards accumulated terminal content.', async (t, terminal) => {
    await echo(t, terminal);
    await quiescent(t, terminal);
    const { client, channel } = terminal;
    await accepted(client, channel, { type: 'terminal/cleared' });
    assert.deepEqual((await state(client, channel)).content, []);
  }),
  terminalTest('terminal/claimed', 'claim-transfer-catalogue', 'An owner can transfer a terminal claim and root metadata reflects the new owner.', async (t, terminal) => {
    const { client, channel } = terminal;
    const observer = await t.client();
    await observer.subscribe(channel);
    const claim = clientClaim(observer.clientId);
    await accepted(client, channel, { type: 'terminal/claimed', claim });
    assert.deepEqual((await state(observer, channel)).claim, claim);
    const info = await catalogue(observer, channel, (entry) => entry.claim.kind === 'client' && entry.claim.clientId === observer.clientId);
    assert.deepEqual(info.claim, claim);
  }),
  terminalTest('terminal/claimed', 'nonholder-claim-rejected', 'SHOULD: A client that does not hold a terminal claim cannot transfer it.', async (t, terminal) => {
    const { client, channel, initial } = terminal;
    const other = await t.client();
    await other.subscribe(channel);
    const after = other.lastServerSeq;
    other.dispatch(channel, { type: 'terminal/claimed', claim: clientClaim(other.clientId) });
    await other.request('ping', { channel: rootChannel });
    const stolen = (event: ActionEnvelope) => event.channel === channel && event.serverSeq > after && !event.rejectionReason
      && event.action.type === 'terminal/claimed' && event.action.claim.kind === 'client' && event.action.claim.clientId === other.clientId;
    assert.ok(!other.actions(channel).some(stolen), 'The nonholder claim was accepted.');
    await other.expectNoNotification<ActionEnvelope>('action', stolen, 350);
    assert.deepEqual((await state(client, channel)).claim, initial.claim);
  }, '/reference/terminal#terminalclaimedaction'),
];
