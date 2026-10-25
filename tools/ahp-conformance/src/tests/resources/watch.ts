import { mkdir, open, unlink, writeFile } from 'node:fs/promises';
import { setTimeout as delay } from 'node:timers/promises';
import { join } from 'node:path';
import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope, CreateResourceWatchParams, CreateResourceWatchResult, ReconnectResult, ResourceWatchState } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import type { Client, TestContext, TestDefinition } from '../../harness/types.ts';
import { hasChange, resourceClient, resourceTest, rootChannel, scratchFile } from './helpers.ts';

const spec = '/reference/resource-watch#createresourcewatch';
type WatchOptions = Pick<CreateResourceWatchParams, 'recursive' | 'includes' | 'excludes'>;

async function watch(t: TestContext, options: WatchOptions = {}, uri = t.scratchUri()) {
  const client = await resourceClient(t);
  const { channel } = await client.request<CreateResourceWatchResult>('createResourceWatch', { channel: rootChannel, uri, ...options });
  let subscribed = true;
  const unsubscribe = async () => {
    if (!subscribed) return;
    subscribed = false;
    await client.unsubscribe(channel);
  };
  t.defer(unsubscribe);
  await client.subscribe(channel);
  const disconnect = async () => {
    subscribed = false;
    await client.close();
  };
  return { client, channel, unsubscribe, disconnect };
}

async function observeChange(t: TestContext, client: Client, channel: string, uri: string, mutate: () => Promise<unknown>, type?: 'added' | 'updated' | 'deleted') {
  const after = client.lastServerSeq;
  let received = false;
  const event = client.waitForAction(channel, (envelope) => envelope.serverSeq > after && hasChange(envelope, uri, type));
  const observed = event.then(() => { received = true; });
  // Subscription has no watcher-ready acknowledgement. Repeat idempotent stimuli until delivery is observed.
  while (!received) {
    await mutate();
    await Promise.race([observed, delay(250, undefined, { signal: t.signal })]);
  }
  return event;
}

async function activeWatch(t: TestContext, options: WatchOptions = {}) {
  const sentinel = scratchFile(t, 'watch-ready');
  await writeFile(sentinel.path, 'before subscription');
  const watched = await watch(t, options);
  let revision = 0;
  await observeChange(t, watched.client, watched.channel, sentinel.uri,
    () => writeFile(sentinel.path, `watch-ready-${++revision}`));
  return watched;
}

export const tests: TestDefinition[] = [
  resourceTest('createResourceWatch', 'file-root', 'A resource watch reports an update to the watched file itself.', async (t) => {
    const file = scratchFile(t);
    await writeFile(file.path, 'before');
    const { client, channel } = await watch(t, {}, file.uri);
    let revision = 0;
    await observeChange(t, client, channel, file.uri, () => writeFile(file.path, String(++revision)), 'updated');
  }, spec),
  ...(['remaining-subscriber', 'creator-disconnect-with-observer'] as const).map((name) => resourceTest(
    'createResourceWatch', name,
    name === 'remaining-subscriber'
      ? 'A remaining resource-watch subscriber receives changes after another subscriber unsubscribes.'
      : 'A resource watch keeps delivering to an observer after its creator disconnects.',
    async (t) => {
      const file = scratchFile(t);
      await writeFile(file.path, 'before');
      const watched = await watch(t);
      const observer = await t.client({ initialSubscriptions: [] });
      await observer.subscribe(watched.channel);
      t.defer(() => observer.unsubscribe(watched.channel));
      let revision = 0;
      await observeChange(t, observer, watched.channel, file.uri, () => writeFile(file.path, String(++revision)));
      if (name === 'remaining-subscriber') {
        await watched.unsubscribe();
        await watched.client.request('ping', { channel: rootChannel });
      } else {
        await watched.disconnect();
      }
      await observer.request('ping', { channel: rootChannel });
      const afterDeparture = scratchFile(t, 'after-departure');
      await observeChange(t, observer, watched.channel, afterDeparture.uri, () => writeFile(afterDeparture.path, String(++revision)));
    }, '/specification/resource-watch-channel#lifecycle',
  )),
  resourceTest('createResourceWatch', 'reconnect-descriptor', 'Snapshot recovery returns the original descriptor for a still-subscribed resource watch.', async (t) => {
    const watched = await watch(t, { recursive: true, includes: { items: ['*.txt'] }, excludes: { items: ['ignored.txt'] } });
    const before = structuredClone(watched.client.state<ResourceWatchState>(watched.channel));
    const observer = await t.client({ initialSubscriptions: [] });
    await observer.subscribe(watched.channel);
    t.defer(() => observer.unsubscribe(watched.channel));
    const clientId = watched.client.clientId;
    await watched.disconnect();
    const resumed = await t.connect({ label: 'watch-reconnect' });
    const result = await resumed.request<ReconnectResult>('reconnect', {
      channel: rootChannel, clientId, lastSeenServerSeq: 0, subscriptions: [watched.channel],
    });
    t.defer(() => resumed.notify('unsubscribe', { channel: watched.channel }));
    if (result.type === 'replay') t.skip('The host selected replay rather than a resource-watch reconnect snapshot.');
    assert.equal(result.type, 'snapshot');
    const snapshot = result.snapshots.find((entry) => entry.resource === watched.channel);
    if (!snapshot) return t.skip('The host did not resume the resource watch on the reconnected peer.');
    assert.deepEqual(snapshot.state, before);
  }, '/specification/resource-watch-channel#state'),
  resourceTest('createResourceWatch', 'includes-excludes-intersection', 'A resource watch excludes paths matching both filters while reporting include-only paths.', async (t) => {
    const ignored = scratchFile(t, 'ignored');
    const included = scratchFile(t, 'included');
    await writeFile(ignored.path, 'before');
    await writeFile(included.path, 'before');
    const { client, channel } = await watch(t, { includes: { items: ['*.txt'] }, excludes: { items: [ignored.name] } });
    let revision = 0;
    await observeChange(t, client, channel, included.uri, async () => {
      await writeFile(ignored.path, String(++revision));
      await writeFile(included.path, String(revision));
    });
    assert.ok(!client.actions(channel).some((event) => hasChange(event, ignored.uri)));
    await client.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === channel && hasChange(event, ignored.uri), 400);
  }, spec),
  resourceTest('createResourceWatch', 'fresh-channel', 'createResourceWatch allocates a distinct resource-watch channel for each successful call.', async (t) => {
    const client = await resourceClient(t);
    const first = await client.request<CreateResourceWatchResult>('createResourceWatch', { channel: rootChannel, uri: t.scratchUri() });
    await client.subscribe(first.channel);
    t.defer(() => client.unsubscribe(first.channel));
    const second = await client.request<CreateResourceWatchResult>('createResourceWatch', { channel: rootChannel, uri: t.scratchUri() });
    await client.subscribe(second.channel);
    t.defer(() => client.unsubscribe(second.channel));
    assert.notEqual(first.channel, second.channel);
  }, '/specification/resource-watch-channel#uri'),
  resourceTest('createResourceWatch', 'default-descriptor', 'A resource-watch subscription describes its root and defaults recursive to false.', async (t) => {
    const { client, channel } = await watch(t);
    const state = client.state<ResourceWatchState>(channel);
    assert.ok(state);
    assert.equal(state.root, t.scratchUri());
    assert.equal(state.recursive, false);
  }, '/specification/resource-watch-channel#state'),
  resourceTest('createResourceWatch', 'explicit-descriptor', 'A resource-watch subscription preserves the supplied recursion and filter settings.', async (t) => {
    const options = { recursive: true, includes: { items: ['*.txt'] }, excludes: { items: ['ignored.txt'] } };
    const { client, channel } = await watch(t, options);
    const state = client.state<ResourceWatchState>(channel);
    assert.ok(state);
    assert.deepEqual({ recursive: state.recursive, includes: state.includes, excludes: state.excludes }, options);
  }, '/specification/resource-watch-channel#state'),
  resourceTest('createResourceWatch', 'added-file', 'A subscribed resource watch reports an added file created through the shared filesystem.', async (t) => {
    const { client, channel } = await activeWatch(t);
    const file = scratchFile(t);
    const handle = await open(file.path, 'wx');
    await handle.close();
    await client.waitForAction(channel, (event) => hasChange(event, file.uri, 'added'));
  }, '/reference/resource-watch#resourcewatchchangedaction'),
  resourceTest('createResourceWatch', 'updated-file', 'A subscribed resource watch reports updates to an existing file.', async (t) => {
    const file = scratchFile(t);
    await writeFile(file.path, 'before');
    const { client, channel } = await watch(t);
    let revision = 0;
    await observeChange(t, client, channel, file.uri,
      () => writeFile(file.path, `after-${++revision}`), 'updated');
  }, '/reference/resource-watch#resourcewatchchangedaction'),
  resourceTest('createResourceWatch', 'deleted-file', 'A subscribed resource watch reports deletion of an existing file.', async (t) => {
    const file = scratchFile(t);
    await writeFile(file.path, 'before');
    const { client, channel } = await activeWatch(t);
    await unlink(file.path);
    await client.waitForAction(channel, (event) => hasChange(event, file.uri, 'deleted'));
  }, '/reference/resource-watch#resourcewatchchangedaction'),
  resourceTest('createResourceWatch', 'recursive-descendant', 'A recursive resource watch reports changes below a nested directory.', async (t) => {
    const directory = t.uniqueId('nested');
    await mkdir(join(t.scratchDir, directory));
    const { client, channel } = await activeWatch(t, { recursive: true });
    const relative = `${directory}/${t.uniqueId('descendant')}.txt`;
    const handle = await open(join(t.scratchDir, relative), 'wx');
    await handle.close();
    await client.waitForAction(channel, (event) => hasChange(event, t.scratchUri(relative), 'added'));
  }, spec),
  resourceTest('createResourceWatch', 'nonrecursive-descendant', 'A non-recursive resource watch does not report files beneath nested directories.', async (t) => {
    const directory = t.uniqueId('nested');
    await mkdir(join(t.scratchDir, directory));
    const relative = `${directory}/${t.uniqueId('descendant')}.txt`;
    const sentinel = scratchFile(t, 'sentinel');
    await writeFile(join(t.scratchDir, relative), 'nested');
    await writeFile(sentinel.path, 'direct child');
    const { client, channel } = await watch(t);
    let revision = 0;
    await observeChange(t, client, channel, sentinel.uri, async () => {
      await writeFile(join(t.scratchDir, relative), `nested-${++revision}`);
      await writeFile(sentinel.path, `direct child-${revision}`);
    });
    assert.ok(!client.actions(channel).some((event) => hasChange(event, t.scratchUri(relative))));
    await client.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === channel && hasChange(event, t.scratchUri(relative)), 400);
  }, spec),
  resourceTest('createResourceWatch', 'excludes', 'A resource watch omits changes matching an excluded relative path.', async (t) => {
    const ignored = scratchFile(t, 'ignored');
    const included = scratchFile(t, 'included');
    await writeFile(ignored.path, 'before');
    await writeFile(included.path, 'before');
    const { client, channel } = await watch(t, { excludes: { items: [ignored.name] } });
    let revision = 0;
    await observeChange(t, client, channel, included.uri, async () => {
      await writeFile(ignored.path, `ignored-${++revision}`);
      await writeFile(included.path, `included-${revision}`);
    });
    assert.ok(!client.actions(channel).some((event) => hasChange(event, ignored.uri)));
    await client.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === channel && hasChange(event, ignored.uri), 400);
  }, spec),
  resourceTest('createResourceWatch', 'includes', 'A resource watch restricts changes to included relative paths.', async (t) => {
    const ignored = scratchFile(t, 'ignored');
    const included = scratchFile(t, 'included');
    await writeFile(ignored.path, 'before');
    await writeFile(included.path, 'before');
    const { client, channel } = await watch(t, { includes: { items: [included.name] } });
    let revision = 0;
    await observeChange(t, client, channel, included.uri, async () => {
      await writeFile(ignored.path, `ignored-${++revision}`);
      await writeFile(included.path, `included-${revision}`);
    });
    assert.ok(!client.actions(channel).some((event) => hasChange(event, ignored.uri)));
    await client.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === channel && hasChange(event, ignored.uri), 400);
  }, spec),
  resourceTest('createResourceWatch', 'nonempty-batches', 'A resource watch never dispatches an empty changes batch.', async (t) => {
    const file = scratchFile(t);
    await writeFile(file.path, 'before');
    const { client, channel } = await watch(t);
    let revision = 0;
    await observeChange(t, client, channel, file.uri, () => writeFile(file.path, `contents-${++revision}`));
    const emptyBatch = (event: ActionEnvelope) => event.channel === channel
      && event.action.type === 'resourceWatch/changed' && event.action.changes.items.length === 0;
    assert.ok(!client.actions(channel).some(emptyBatch));
    await client.expectNoNotification<ActionEnvelope>('action', emptyBatch, 400);
  }, '/reference/resource-watch#resourcewatchchangedaction'),
  resourceTest('createResourceWatch', 'immutable-mirror', 'Resource-watch changes preserve descriptor state and its reducer mirror matches a fresh snapshot.', async (t) => {
    const file = scratchFile(t);
    await writeFile(file.path, 'before');
    const { client, channel } = await watch(t, { recursive: true });
    const before = structuredClone(client.state<ResourceWatchState>(channel));
    let revision = 0;
    await observeChange(t, client, channel, file.uri, () => writeFile(file.path, `contents-${++revision}`));
    assert.deepEqual(client.state<ResourceWatchState>(channel), before);
    await client.assertMirrorMatchesSnapshot(channel);
  }, '/specification/resource-watch-channel#state'),
  resourceTest('createResourceWatch', 'unsubscribe', 'Unsubscribing the last resource-watch subscriber stops delivery of subsequent changes.', async (t) => {
    const before = scratchFile(t, 'before-unsubscribe');
    await writeFile(before.path, 'before');
    const { client, channel, unsubscribe } = await watch(t);
    let revision = 0;
    await observeChange(t, client, channel, before.uri, () => writeFile(before.path, `contents-${++revision}`));
    await unsubscribe();
    const after = scratchFile(t, 'after-unsubscribe');
    await writeFile(after.path, 'contents');
    await client.expectNoNotification<ActionEnvelope>('action', (event) => event.channel === channel && hasChange(event, after.uri), 400);
  }, '/specification/resource-watch-channel#lifecycle'),
  resourceTest('createResourceWatch', 'missing-root', 'createResourceWatch returns NotFound for a missing watch root.', async (t) => {
    const client = await resourceClient(t);
    await expectRpcError(client.request('createResourceWatch', { channel: rootChannel, uri: scratchFile(t, 'missing', '').uri }), AhpErrorCodes.NotFound);
  }, spec),
];
