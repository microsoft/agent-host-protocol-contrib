import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import type { SessionState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { Client, Requirement, TestContext, TestDefinition } from '../../harness/types.ts';
import { dispatchAccepted, dispatchAndWait } from './helpers.ts';

const primaryReplacement: Requirement = {
  label: 'multipleWorkingDirectories.primaryReplacement', kind: 'capability',
  check: (profile) => profile.agent.capabilities?.multipleWorkingDirectories?.primaryReplacement === true,
};
const immutablePrimary: Requirement = {
  label: 'multipleWorkingDirectories.immutablePrimary', kind: 'capability',
  check: (profile) => profile.agent.capabilities?.multipleWorkingDirectories?.immutablePrimary === true &&
    profile.agent.capabilities?.multipleWorkingDirectories?.primaryReplacement !== true,
};

function directoryTest(definition: Omit<TestDefinition, 'area'>): TestDefinition {
  return {
    ...definition, area: 'Tools & permissions',
    requires: [requires.multipleWorkingDirectories, requires.sharedFilesystem, ...(definition.requires ?? [])],
  };
}

async function directories(t: TestContext, count: number): Promise<string[]> {
  return Promise.all(Array.from({ length: count }, async () => {
    const name = t.uniqueId('directory');
    await mkdir(join(t.scratchDir, name));
    return t.scratchUri(name);
  }));
}

async function fixture(t: TestContext, initialCount = 1, spareCount = 1) {
  const dirs = await directories(t, initialCount + spareCount);
  const initial = dirs.slice(0, initialCount);
  const client = await t.client();
  const session = await t.session(client, { workingDirectories: initial });
  return { client, session, dirs, initial };
}

async function snapshotDirectories(client: Client, uri: string) {
  return ((await client.subscribe(uri)).state as SessionState).workingDirectories;
}

async function replaceDirectory(client: Client, uri: string, directory: string, replacement: string, expected: string[]) {
  const before = await snapshotDirectories(client, uri);
  const envelope = await dispatchAndWait(client, uri, {
    type: 'session/workingDirectoryReplaced', directory, replacement,
  });
  // Replacement may be rejected if the backend cannot apply the change.
  assert.deepEqual(await snapshotDirectories(client, uri), envelope.rejectionReason === undefined ? expected : before);
}

export const tests: TestDefinition[] = [
  {
    id: 'tools/working-directories/creation-single-directory', area: 'Tools & permissions', feature: 'workingDirectories',
    title: 'The session snapshot retains the working directory supplied at creation.',
    spec: '/reference/session#createsession',
    async run(t) {
      const client = await t.client();
      const expected = [t.scratchUri()];
      const session = await t.session(client, { workingDirectories: expected });
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri), expected);
    },
  },
  directoryTest({
    id: 'tools/working-directories/creation-multiple-directories', feature: 'workingDirectories',
    title: 'A multiroot session snapshot retains all directories in their supplied order.',
    spec: '/reference/session#createsession',
    async run(t) {
      const { client, session, initial } = await fixture(t, 2, 0);
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri), initial);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/add-directory', feature: 'workingDirectorySet',
    title: 'Adding a working directory appends it to session state.',
    spec: '/reference/session#sessionworkingdirectorysetaction',
    async run(t) {
      const { client, session, dirs } = await fixture(t);
      await session.ready();
      const before = await snapshotDirectories(client, session.sessionUri) ?? [];
      await dispatchAccepted(client, session.sessionUri, {
        type: 'session/workingDirectorySet', directory: dirs[1],
      });
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri), [...before, dirs[1]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/add-existing-directory-no-op', feature: 'workingDirectorySet',
    title: 'Adding an existing working directory does not duplicate it.',
    spec: '/reference/session#sessionworkingdirectorysetaction',
    async run(t) {
      const { client, session } = await fixture(t, 1, 0);
      await session.ready();
      const before = await snapshotDirectories(client, session.sessionUri);
      assert(before?.length, 'no existing working directory to add');
      await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectorySet', directory: before[0],
      });
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri), before);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/remove-secondary-directory', feature: 'workingDirectoryRemoved',
    title: 'An accepted directory removal removes the secondary directory from session state.',
    spec: '/reference/session#sessionworkingdirectoryremovedaction',
    async run(t) {
      const { client, session, dirs, initial } = await fixture(t, 2, 0);
      const envelope = await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectoryRemoved', directory: dirs[1],
      });
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri),
        envelope.rejectionReason === undefined ? [dirs[0]] : initial);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/remove-absent-directory-no-op', feature: 'workingDirectoryRemoved',
    title: 'Removing an absent working directory leaves the directory set unchanged.',
    spec: '/reference/session#sessionworkingdirectoryremovedaction',
    async run(t) {
      const { client, session, dirs, initial } = await fixture(t);
      await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectoryRemoved', directory: dirs[1],
      });
      assert.deepEqual(await snapshotDirectories(client, session.sessionUri), initial);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/replace-secondary-directory', feature: 'workingDirectoryReplaced',
    title: 'An accepted secondary-directory replacement preserves all other directory positions.',
    spec: '/reference/session#sessionworkingdirectoryreplacedaction',
    async run(t) {
      const { client, session, dirs } = await fixture(t, 3, 1);
      await replaceDirectory(client, session.sessionUri, dirs[1], dirs[3], [dirs[0], dirs[3], dirs[2]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/replace-absent-directory-no-op', feature: 'workingDirectoryReplaced',
    title: 'Replacing an absent working directory leaves the directory set unchanged.',
    spec: '/reference/session#sessionworkingdirectoryreplacedaction',
    async run(t) {
      const { client, session, dirs, initial } = await fixture(t, 1, 2);
      await replaceDirectory(client, session.sessionUri, dirs[1], dirs[2], initial);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/replacement-deduplicates-later-entry', feature: 'workingDirectoryReplaced',
    title: 'Replacing a directory with a later entry moves that entry to the target position without duplication.',
    spec: '/reference/session#sessionworkingdirectoryreplacedaction',
    async run(t) {
      const { client, session, dirs } = await fixture(t, 3, 0);
      await replaceDirectory(client, session.sessionUri, dirs[1], dirs[2], [dirs[0], dirs[2]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/replacement-deduplicates-earlier-entry', feature: 'workingDirectoryReplaced',
    title: 'Replacing a directory with an earlier entry preserves the earlier entry position without duplication.',
    spec: '/reference/session#sessionworkingdirectoryreplacedaction',
    async run(t) {
      const { client, session, dirs } = await fixture(t, 3, 0);
      await replaceDirectory(client, session.sessionUri, dirs[2], dirs[0], [dirs[0], dirs[1]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/replace-primary-directory', feature: 'workingDirectoryReplaced',
    title: 'An accepted primary-directory replacement changes only the protected primary slot.',
    spec: '/reference/session#sessionworkingdirectoryreplacedaction', requires: [primaryReplacement],
    async run(t) {
      const { client, session, dirs } = await fixture(t, 2, 1);
      await replaceDirectory(client, session.sessionUri, dirs[0], dirs[2], [dirs[2], dirs[1]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/reject-protected-primary-removal', feature: 'workingDirectoryRemoved',
    title: 'The host rejects generic removal of a protected replaceable primary directory.',
    spec: '/reference/session#sessionworkingdirectoryremovedaction', requires: [primaryReplacement],
    async run(t) {
      const { client, session, dirs } = await fixture(t, 2, 0);
      const envelope = await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectoryRemoved', directory: dirs[0],
      });
      assert(typeof envelope.rejectionReason === 'string' && envelope.rejectionReason.length > 0);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/immutable-primary-preserved', feature: 'workingDirectoryReplaced',
    title: 'Replacing a secondary directory leaves an immutable primary fixed in the first position.',
    spec: '/guide/state-model#multiroot-sessions', requires: [immutablePrimary],
    async run(t) {
      const { client, session, dirs } = await fixture(t, 2, 1);
      await replaceDirectory(client, session.sessionUri, dirs[1], dirs[2], [dirs[0], dirs[2]]);
    },
  }),
  directoryTest({
    id: 'tools/working-directories/mirror-matches-snapshot', feature: 'workingDirectories',
    title: 'The session mirror matches a fresh snapshot after working-directory actions, including rejections.',
    spec: '/guide/actions#reducers',
    async run(t) {
      const { client, session, dirs } = await fixture(t);
      await session.ready();
      await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectorySet', directory: dirs[1],
      });
      await dispatchAndWait(client, session.sessionUri, {
        type: 'session/workingDirectoryRemoved', directory: dirs[1],
      });
      await client.assertMirrorMatchesSnapshot(session.sessionUri);
    },
  }),
];
