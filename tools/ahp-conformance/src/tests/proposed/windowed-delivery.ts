import { assert } from '../../harness/assert.ts';
import { proposedClient } from '../../harness/proposed.ts';
import { requires } from '../../harness/requirements.ts';
import type { TestDefinition } from '../../harness/types.ts';
import { accepted, headSpec, record, snapshot } from './helpers.ts';
import {
  assertLimits, channelOf, dispatchText, fragmentText, offerWindow, reconnectWindow,
  subscribeWindow, windowSetup,
} from './window-helpers.ts';

const spec = (section: string) => headSpec('docs/specification/windowed-delivery.md', section);
const budget = 256 * 1024;

export const tests: TestDefinition[] = [
  {
    id: 'proposed/window/negotiated-limits', area: 'Proposed', feature: 'windowedDelivery',
    title: 'Accepted window limits never exceed the offer and preserve the minimum frame size.',
    spec: spec('negotiation'), requires: [requires.proposed],
    async run(t) {
      const client = await proposedClient(t, { initialSubscriptions: [] });
      const offered = { windowBytes: 4096, maximumFrameBytes: 1024 };
      const { result } = await offerWindow(client, 'ahp-root://', offered);
      const flow = result.flowControl;
      if (!flow) return t.skip('Host declined flowControl on root subscribe; negotiating 1.1.0 does not advertise windowed delivery');
      assertLimits(flow.clientReceive, offered);
      assertLimits(flow.hostReceive);
      assert.equal(result.snapshot, undefined);
    },
  },
  {
    id: 'proposed/window/snapshot-startup-barrier', area: 'Proposed', feature: 'channel/snapshot',
    title: 'A windowed subscription starts with a snapshot after its subscribe response.',
    spec: spec('initial-data'), requires: [requires.proposed],
    async run(t) {
      const client = await proposedClient(t, { initialSubscriptions: [] });
      const peer = await subscribeWindow(t, client, 'ahp-root://', { windowBytes: 1024 * 1024, maximumFrameBytes: 65536 });
      const initial = await peer.initialSnapshot();
      assert.equal(initial.resource, 'ahp-root://');
      assert.ok(Array.isArray(record(initial.state).agents));
    },
  },
  {
    id: 'proposed/window/soft-window-stalls-and-resumes', area: 'Proposed', feature: 'channel/credit',
    title: 'A full soft window stops data until cumulative credit permits another message.',
    spec: spec('window-and-credit'), requires: [requires.proposed],
    async run(t) {
      const { peer } = await windowSetup(t);
      await peer.paused();
      await peer.quiet();
      const count = peer.dataFrames.length;
      peer.credit();
      await peer.until(() => peer.dataFrames.length > count, false);
      assert.ok(peer.outstanding < peer.flow.clientReceive.windowBytes + peer.flow.clientReceive.maximumFrameBytes);
    },
  },
  {
    id: 'proposed/window/receiver-counts-wire-text', area: 'Proposed', feature: 'channel/credit',
    title: 'Host credit makes progress using actual UTF-16 wire text including envelopes and escaping.',
    spec: spec('measuring-size'), requires: [requires.proposed],
    async run(t) {
      const { peer, owner, session } = await windowSetup(t, {
        receive: { windowBytes: 1024 * 1024, maximumFrameBytes: 65536 },
      });
      await peer.initialSnapshot();
      peer.credit();
      if (peer.flow.hostReceive.windowBytes > budget) t.skip('Negotiated host receive window exceeds the bounded Unicode accounting payload budget');
      const title = `wire é😀\\\" ${'A é 😀 \\ "\n'.repeat(Math.ceil(peer.flow.hostReceive.windowBytes / 5))}`;
      const joined = JSON.stringify({ jsonrpc: '2.0', method: 'dispatchAction', params: {
        channel: session.sessionUri, clientSeq: 1, action: { type: 'session/titleChanged', title },
      } }, null, 2);
      const chunks = fragmentText(session.sessionUri, joined, peer.flow.hostReceive.maximumFrameBytes - 8);
      for (let index = 0; index < chunks.length; index++) {
        const frame = JSON.stringify({ jsonrpc: '2.0', method: 'channel/frame', params: {
          channel: session.sessionUri, data: chunks[index], ...(index === chunks.length - 1 && { final: true }),
        } });
        await peer.sendText(`\n ${frame}\n `);
      }
      await peer.until(() => peer.logical.some((message) => message.method === 'action'
        && record(record(message.params).action).title === title));
      await peer.until(() => peer.hostConsumedBytes > 0);
      assert.ok(peer.hostConsumedBytes > 0, 'host never released incoming fragment credit');
      assert.ok(peer.hostConsumedBytes <= peer.sentBytes, 'host over-credited received UTF-16 bytes');
      assert.equal((await snapshot(owner, session.sessionUri)).state.title, title);
    },
  },
  {
    id: 'proposed/window/outgoing-fragment-integrity', area: 'Proposed', feature: 'channel/frame',
    title: 'Oversized Unicode snapshots use nonempty lossless size-bounded fragments.',
    spec: spec('fragments'), requires: [requires.proposed],
    async run(t) {
      const { peer, title } = await windowSetup(t);
      const initial = await peer.initialSnapshot();
      assert.equal(record(initial.state).title, title);
      assert.ok(peer.dataFrames.some((message) => message.method === 'channel/frame'), 'oversized indivisible snapshot was not fragmented');
    },
  },
  {
    id: 'proposed/window/incoming-fragment-atomicity', area: 'Proposed', feature: 'channel/frame',
    title: 'An incoming fragmented action applies exactly once and only after its final fragment.',
    spec: spec('fragments'), requires: [requires.proposed],
    async run(t) {
      const { peer, owner, client, session, title: before } = await windowSetup(t);
      await peer.initialSnapshot();
      peer.credit();
      const title = `${t.uniqueId('fragmented-title')} ${'😀é\\"'.repeat(256)}`;
      const chunks = fragmentText(session.sessionUri, dispatchText(session.sessionUri, 1, title), Math.min(1024, peer.flow.hostReceive.maximumFrameBytes));
      assert.ok(chunks.length >= 2);
      await peer.sendFragment(chunks[0]!);
      await client.expectNoNotification('action', (params) => {
        const value = record(params);
        return value.channel === session.sessionUri && record(value.action).title === title;
      }, 350);
      assert.equal((await snapshot(owner, session.sessionUri)).state.title, before);
      peer.poll();
      assert.ok(!peer.logical.some((message) => message.method === 'action' && record(record(message.params).action).title === title), 'host echoed the joined action before its final fragment');
      for (let index = 1; index < chunks.length; index++) await peer.sendFragment(chunks[index]!, index === chunks.length - 1);
      await peer.until(() => peer.logical.some((message) => message.method === 'action' && record(record(message.params).action).title === title));
      await client.request('ping', { channel: 'ahp-root://' });
      peer.poll();
      assert.equal(peer.logical.filter((message) => message.method === 'action' && record(record(message.params).action).title === title).length, 1);
      assert.equal((await snapshot(owner, session.sessionUri)).state.title, title);
    },
  },
  {
    id: 'proposed/window/incomplete-message-credit-progress', area: 'Proposed', feature: 'channel/credit',
    title: 'The host releases fragment credit before a message larger than its window is complete.',
    spec: spec('window-and-credit'), requires: [requires.proposed],
    async run(t) {
      const { peer, session } = await windowSetup(t);
      await peer.initialSnapshot();
      peer.credit();
      if (peer.flow.hostReceive.windowBytes > budget) t.skip('Host receive window exceeds safe incomplete-message fixture budget');
      const title = `${t.uniqueId('large-incoming')} ${'é😀'.repeat(peer.flow.hostReceive.windowBytes)}`;
      const chunks = fragmentText(session.sessionUri, dispatchText(session.sessionUri, 1, title),
        Math.min(peer.flow.hostReceive.maximumFrameBytes, Math.max(1024, Math.ceil(peer.flow.hostReceive.windowBytes / 2))));
      assert.ok(chunks.length >= 2);
      for (let index = 0; index < chunks.length - 1; index++) await peer.sendFragment(chunks[index]!);
      peer.poll();
      assert.ok(peer.sentBytes > peer.flow.hostReceive.windowBytes);
      assert.ok(peer.hostConsumedBytes > 0, 'host waited for final fragment before crediting an oversized message');
      await peer.sendFragment(chunks.at(-1)!, true);
      await peer.until(() => peer.logical.some((message) => message.method === 'action' && record(record(message.params).action).title === title));
    },
  },
  {
    id: 'proposed/window/control-exempt-notifications', area: 'Proposed', feature: 'windowExempt',
    title: 'Credit and unsubscribe bypass a full subscription data window.',
    spec: spec('control-messages'), requires: [requires.proposed],
    async run(t) {
      const { peer, owner, client, session } = await windowSetup(t);
      await peer.paused();
      if (peer.flow.hostReceive.windowBytes > budget) t.skip('Host receive window exceeds safe blocked-window control fixture budget');
      const text = dispatchText(session.sessionUri, 1, 'x'.repeat(peer.flow.hostReceive.windowBytes * 3));
      await peer.sendJoined(text, false);
      await peer.until(() => peer.hostConsumedBytes > 0, false);
      assert.ok(peer.outstanding >= peer.flow.clientReceive.windowBytes, 'data window was not full while host sent credit');
      await peer.quiet();
      client.notify('unsubscribe', { channel: session.sessionUri });
      await client.request('ping', { channel: 'ahp-root://' });
      const marker = t.uniqueId('after-unsubscribe');
      await accepted(owner, session.sessionUri, { type: 'session/titleChanged', title: marker });
      await peer.quiet();
    },
  },
  {
    id: 'proposed/window/subscription-credit-isolation', area: 'Proposed', feature: 'channel/credit',
    title: 'A second observer’s credit cannot unblock a different full subscription.',
    spec: spec('negotiation'), requires: [requires.proposed],
    async run(t) {
      const { peer: held, owner, session } = await windowSetup(t);
      await held.paused();
      const second = await proposedClient(t, { initialSubscriptions: [] });
      const drained = await subscribeWindow(t, second, session.sessionUri);
      await drained.initialSnapshot();
      drained.credit();
      const marker = t.uniqueId('independent-credit');
      await accepted(owner, session.sessionUri, { type: 'session/titleChanged', title: marker });
      await drained.until(() => drained.logical.some((message) => message.method === 'action'
        && record(record(message.params).action).title === marker));
      drained.credit();
      await held.quiet();
    },
  },
  {
    id: 'proposed/window/reconnect-response-partition', area: 'Proposed', feature: 'reconnect',
    title: 'Reconnect partitions requested windows from ordinary actions and snapshots after its response.',
    spec: spec('reconnection'), requires: [requires.proposed],
    async run(t) {
      const { peer, owner, client, session } = await windowSetup(t);
      const initial = await peer.initialSnapshot();
      peer.credit();
      await snapshot(client, 'ahp-root://');
      await client.close();
      await accepted(owner, session.sessionUri, { type: 'session/titleChanged', title: t.uniqueId('offline') });
      const connection = await t.connect({ label: 'proposed-partition-reconnect' });
      const result = record(await connection.request('reconnect', {
        channel: 'ahp-root://', clientId: client.clientId, lastSeenServerSeq: initial.fromSeq,
        subscriptions: ['ahp-root://', session.sessionUri], windows: { items: [{
          channel: session.sessionUri, flowControl: { receive: peer.flow.clientReceive }, lastAppliedServerSeq: initial.fromSeq,
        }] },
      }));
      const response = connection.received.findIndex((message) => message.result === result);
      assert.ok(response >= 0);
      assert.ok(connection.received.slice(0, response).every((message) => channelOf(message) !== session.sessionUri), 'window traffic preceded reconnect result');
      if (result.type === 'replay') {
        assert.ok(Array.isArray(result.actions));
        for (const action of result.actions as Record<string, unknown>[]) assert.notEqual(action.channel, session.sessionUri);
      } else {
        assert.equal(result.type, 'snapshot');
        assert.ok(Array.isArray(result.snapshots));
        for (const value of result.snapshots as Record<string, unknown>[]) assert.notEqual(value.resource, session.sessionUri);
      }
      if (result.windows !== undefined) {
        const items = record(result.windows).items;
        assert.ok(Array.isArray(items));
        const channels = (items as Record<string, unknown>[]).map((entry) => entry.channel);
        assert.ok(channels.every((channel) => channel === session.sessionUri), 'host returned an unrequested window');
        assert.equal(new Set(channels).size, channels.length);
      }
      const ordinary = await proposedClient(t, { initialSubscriptions: [] });
      await ordinary.close();
      const replacement = await t.connect({ label: 'proposed-ordinary-reconnect' });
      const plain = record(await replacement.request('reconnect', {
        channel: 'ahp-root://', clientId: ordinary.clientId, lastSeenServerSeq: ordinary.lastServerSeq, subscriptions: [],
      }));
      assert.equal(plain.windows, undefined, 'host returned windows without a windows request');
    },
  },
  {
    id: 'proposed/window/reconnect-fresh-byte-counters', area: 'Proposed', feature: 'reconnect',
    title: 'A kept window restarts byte accounting at zero with newly negotiated limits.',
    spec: spec('reconnection'), requires: [requires.proposed],
    async run(t) {
      const { peer, owner, client, session } = await windowSetup(t);
      await peer.paused();
      const resumed = await reconnectWindow(t, client, session.sessionUri, peer.flow.clientReceive, 0);
      const response = resumed.connection.received.findIndex((message) => message.result === resumed.result);
      assert.ok(resumed.connection.received.slice(0, response).every((message) => channelOf(message) !== session.sessionUri));
      await accepted(owner, session.sessionUri, { type: 'session/titleChanged', title: t.uniqueId('after-reconnect') });
      await resumed.peer.until(() => resumed.peer.receivedBytes > 0, false);
      assert.ok(resumed.peer.receivedBytes > 0, 'new connection stayed blocked on old outstanding bytes');
      assert.equal(resumed.peer.releasedBytes, 0, 'test must not credit old-connection bytes');
    },
  },
  {
    id: 'proposed/window/cumulative-credit-is-monotone', area: 'Proposed', feature: 'channel/credit',
    title: 'Repeated and regressed cumulative credit do not grant new sending capacity.',
    spec: spec('window-and-credit'), requires: [requires.proposed],
    async run(t) {
      const { peer } = await windowSetup(t);
      await peer.paused();
      const oldCredit = peer.receivedBytes;
      const before = peer.dataFrames.length;
      peer.credit(oldCredit);
      await peer.until(() => peer.dataFrames.length > before, false);
      await peer.paused();
      peer.credit(oldCredit);
      peer.credit(Math.max(0, oldCredit - 1));
      await peer.quiet();
      const count = peer.dataFrames.length;
      peer.credit();
      await peer.until(() => peer.dataFrames.length > count, false);
    },
  },
  {
    id: 'proposed/window/resubscribe-preserves-byte-counters', area: 'Proposed', feature: 'subscribe',
    title: 'Windowed resubscribe preserves outstanding totals when changing receive limits.',
    spec: spec('negotiation'), requires: [requires.proposed],
    async run(t) {
      const { peer, client, session } = await windowSetup(t);
      await peer.paused();
      const oldBytes = peer.receivedBytes;
      const receive = { ...peer.flow.clientReceive, windowBytes: Math.max(1, Math.floor(peer.flow.clientReceive.windowBytes / 2)) };
      const { result } = await offerWindow(client, session.sessionUri, receive);
      assert.ok(result.flowControl, 'accepted window silently changed to ordinary delivery on resubscribe');
      peer.flow.clientReceive = result.flowControl.clientReceive;
      peer.flow.hostReceive = result.flowControl.hostReceive;
      await peer.quiet();
      peer.credit(oldBytes);
      await peer.until(() => peer.receivedBytes > oldBytes, false);
    },
  },
  {
    id: 'proposed/window/empty-state-still-has-initial-snapshot', area: 'Proposed', feature: 'channel/snapshot',
    title: 'An accepted telemetry window starts with a real empty-state snapshot.',
    spec: spec('initial-data'), requires: [requires.proposed],
    async run(t) {
      const client = await proposedClient(t, { initialSubscriptions: [] });
      const telemetry = client.initializeResult.telemetry;
      const channel = Object.values(telemetry ?? {}).find((value): value is string => typeof value === 'string');
      if (!channel) return t.skip('Proposed initialize result advertises no telemetry channel with empty state');
      const peer = await subscribeWindow(t, client, channel);
      const initial = await peer.initialSnapshot();
      assert.deepEqual(initial.state, {});
      assert.ok((initial.fromSeq as number) >= client.initializeResult.serverSeq);
    },
  },
];
