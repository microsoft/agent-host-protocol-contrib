import { AhpErrorCodes, PROTOCOL_VERSION } from '@microsoft/agent-host-protocol';
import type { RootState } from '@microsoft/agent-host-protocol';
import { assert, expectRpcError } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { TestContext, TestDefinition } from '../../harness/types.ts';
import { assertSnapshot, initialize, ROOT, waitForClose } from './helpers.ts';

// Wire-observable cases from upstream types/test-cases/version-negotiation.json.
const compatible = [
  { name: 'pre-one-baseline', offered: ['0.9.0'], expected: '0.9.0' },
  { name: 'stable-baseline', offered: ['1.0.0'], expected: '1.0.0' },
  { name: 'highest-version-offered-last', offered: ['0.9.0', '1.0.0'], expected: '1.0.0' },
  { name: 'highest-version-offered-first', offered: ['1.0.0', '0.9.0'], expected: '1.0.0' },
  { name: 'numeric-patch-order', offered: ['0.9.8', '0.9.10'], expected: '0.9.10' },
  { name: 'numeric-minor-order', offered: ['1.1.0', '1.10.0', '1.2.99'], expected: '1.10.0' },
  { name: 'numeric-minor-order-permuted', offered: ['1.2.99', '1.10.0', '1.1.0'], expected: '1.10.0' },
  { name: 'duplicate-offered-versions', offered: ['1.0.0', '1.0.0'], expected: '1.0.0' },
];

const unsupported = [
  { name: 'empty-offer', offered: [] },
  { name: 'incompatible-offer', offered: undefined },
];

const malformed = [
  { name: 'missing-patch', offered: ['1.0'] },
  { name: 'extra-component', offered: ['1.0.0.0'] },
  { name: 'leading-zero-major', offered: ['01.0.0'] },
  { name: 'leading-zero-minor', offered: ['1.00.0'] },
  { name: 'leading-zero-patch', offered: ['1.0.01'] },
  { name: 'prerelease', offered: ['1.0.0-beta'] },
  { name: 'build-metadata', offered: ['1.0.0+build'] },
  { name: 'leading-whitespace', offered: [' 1.0.0'] },
  { name: 'trailing-newline', offered: ['1.0.0\n'] },
  { name: 'negative-component', offered: ['1.-1.0'] },
  { name: 'mixed-malformed-offer', offered: ['1.0.0', 'invalid'] },
];

async function incompatibleOffer(t: TestContext): Promise<string[]> {
  const probe = await t.connect({ label: 'supported-versions' });
  let error: unknown;
  try { await initialize(t, probe, { protocolVersions: [] }); }
  catch (cause) { error = cause; }
  if (!(error instanceof RpcError) || error.code !== AhpErrorCodes.UnsupportedProtocolVersion) {
    t.signal.throwIfAborted();
    t.skip('Cannot discover supported versions; rejection of an empty offer is covered by transport/initialize/empty-offer.');
  }
  const supported = (error.data as { supportedVersions?: unknown } | null)?.supportedVersions;
  if (!Array.isArray(supported) || supported.length === 0 ||
      !supported.every((version) => typeof version === 'string' && /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.test(version))) {
    t.skip('Cannot derive an incompatible offer from supportedVersions; range advertisements and missing baselines need separate handling.');
  }
  // Concrete advertised baselines imply caret ranges; development baselines may be additional.
  const baselines = (supported as string[]).map((version) => version.split('.').map(Number));
  const major = Math.max(...baselines.map((version) => version[0]!)) + 1;
  const minor = Math.max(0, ...baselines.filter((version) => version[0] === 0).map((version) => version[1]!)) + 1;
  if (!Number.isSafeInteger(major) || !Number.isSafeInteger(minor)) {
    t.skip('Cannot derive an incompatible offer outside the advertised numeric baselines.');
  }
  return [`${major}.0.0`, `0.${minor}.0`];
}

const versionTests: TestDefinition[] = [
  {
    id: 'transport/initialize/skip-incompatible-versions',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Version negotiation skips versions outside the advertised ranges and selects the offered stable baseline.',
    spec: '/specification/versioning#negotiation',
    async run(t) {
      const offered = [...await incompatibleOffer(t), '1.0.0'];
      const connection = await t.connect();
      const result = await initialize(t, connection, { protocolVersions: offered, initialSubscriptions: [] });
      assert.equal(result.protocolVersion, '1.0.0');
    },
  },
  ...compatible.map(({ name, offered, expected }): TestDefinition => ({
    id: `transport/initialize/${name}`,
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: `Version negotiation selects ${expected} from ${JSON.stringify(offered)}.`,
    spec: '/specification/versioning#negotiation',
    async run(t) {
      const connection = await t.connect();
      const result = await initialize(t, connection, { protocolVersions: offered, initialSubscriptions: [] });
      assert.equal(result.protocolVersion, expected);
    },
  })),
  ...unsupported.map(({ name, offered }): TestDefinition => ({
    id: `transport/initialize/${name}`,
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: `Version negotiation rejects ${name === 'empty-offer' ? 'an empty offer' : 'an entirely incompatible offer'} with UnsupportedProtocolVersion.`,
    spec: '/specification/versioning#negotiation',
    async run(t) {
      const versions = offered ?? await incompatibleOffer(t);
      const connection = await t.connect();
      await expectRpcError(initialize(t, connection, { protocolVersions: versions }), AhpErrorCodes.UnsupportedProtocolVersion);
    },
  })),
  ...malformed.map(({ name, offered }): TestDefinition => ({
    id: `transport/initialize/invalid-version-${name}`,
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: `Version negotiation rejects the malformed ${name.replaceAll('-', ' ')} version offer.`,
    spec: '/specification/versioning#negotiation',
    async run(t) {
      const connection = await t.connect();
      await assert.rejects(initialize(t, connection, { protocolVersions: offered }), RpcError);
    },
  })),
  {
    id: 'transport/initialize/unsupported-version-data',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'UnsupportedProtocolVersion includes the supportedVersions array in its error data.',
    spec: '/reference/error-codes#unsupportedprotocolversionerrordata',
    async run(t) {
      const connection = await t.connect();
      const error = await expectRpcError(
        initialize(t, connection, { protocolVersions: [] }),
        AhpErrorCodes.UnsupportedProtocolVersion,
      );
      assert.ok(error.data !== null && typeof error.data === 'object');
      const versions = (error.data as { supportedVersions?: unknown }).supportedVersions;
      assert.ok(Array.isArray(versions));
      assert.ok(versions.length > 0, 'the host must advertise versions it can speak');
      assert.ok(versions.every((version) => typeof version === 'string' && version.length > 0));
    },
  },
  {
    id: 'transport/initialize/unsupported-version-closes-connection',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'The server closes the connection after rejecting incompatible protocol versions.',
    spec: '/specification/versioning#negotiation',
    async run(t) {
      const connection = await t.connect();
      await expectRpcError(
        initialize(t, connection, { protocolVersions: [] }),
        AhpErrorCodes.UnsupportedProtocolVersion,
      );
      await waitForClose(t, connection);
    },
  },
];

export const tests: TestDefinition[] = [
  {
    id: 'transport/initialize/offered-version',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization returns a protocol version from the offered list.',
    spec: '/reference/common#initialize',
    async run(t) {
      const offered = [PROTOCOL_VERSION];
      const connection = await t.connect();
      const result = await initialize(t, connection, { protocolVersions: offered });
      assert.ok(offered.includes(result.protocolVersion));
    },
  },
  {
    id: 'transport/initialize/server-sequence',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization returns a numeric server sequence number.',
    spec: '/reference/common#initialize',
    async run(t) {
      const client = await t.client();
      assert.equal(typeof client.initializeResult.serverSeq, 'number');
      assert.ok(Number.isFinite(client.initializeResult.serverSeq));
    },
  },
  {
    id: 'transport/initialize/root-snapshot-envelope',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization returns a state snapshot for the requested root channel.',
    spec: '/reference/common#snapshot',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [ROOT] });
      assertSnapshot(client.initializeResult.snapshots.find((snapshot) => snapshot.resource === ROOT), ROOT);
    },
  },
  {
    id: 'transport/initialize/root-state-shape',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'The root initialization snapshot contains an agents array.',
    spec: '/reference/root#rootstate',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [ROOT] });
      const snapshot = client.initializeResult.snapshots.find((entry) => entry.resource === ROOT);
      assertSnapshot(snapshot, ROOT);
      assert.ok(Array.isArray((snapshot.state as RootState).agents));
    },
  },
  {
    id: 'transport/initialize/empty-initial-subscriptions',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'An empty initial subscription list produces no snapshots.',
    spec: '/specification/subscriptions#initial-subscriptions',
    async run(t) {
      const client = await t.client({ initialSubscriptions: [] });
      assert.deepEqual(client.initializeResult.snapshots, []);
    },
  },
  {
    id: 'transport/initialize/all-requested-snapshots',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization includes every requested stateful channel snapshot.',
    spec: '/specification/subscriptions#initial-subscriptions',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      const requested = [ROOT, session.sessionUri, session.chatUri];
      const client = await t.client({ initialSubscriptions: requested });
      for (const resource of requested) {
        assertSnapshot(client.initializeResult.snapshots.find((snapshot) => snapshot.resource === resource), resource);
      }
    },
  },
  {
    id: 'transport/initialize/only-requested-snapshots',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization does not return snapshots for unrequested channels.',
    spec: '/specification/subscriptions#initial-subscriptions',
    async run(t) {
      const owner = await t.client();
      const session = await t.session(owner);
      const client = await t.client({ initialSubscriptions: [session.sessionUri] });
      assert.ok(Array.isArray(client.initializeResult.snapshots));
      assert.ok(client.initializeResult.snapshots.every((snapshot) => snapshot.resource === session.sessionUri));
    },
  },
  {
    id: 'transport/initialize/optional-fields-omitted',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization succeeds without optional client fields.',
    spec: '/reference/common#initialize',
    async run(t) {
      const connection = await t.connect();
      const result = await connection.request<{ protocolVersion: string }>('initialize', {
        channel: ROOT,
        protocolVersions: [PROTOCOL_VERSION],
        clientId: t.uniqueId('minimal-client'),
      });
      assert.equal(result.protocolVersion, PROTOCOL_VERSION);
    },
  },
  {
    id: 'transport/initialize/informational-client-identity',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization accepts client implementation identity with a non-SemVer build version.',
    spec: '/specification/lifecycle#implementation-identity',
    async run(t) {
      const connection = await t.connect();
      const result = await initialize(t, connection, {
        clientInfo: { name: t.uniqueId('implementation'), version: 'development-build', title: 'Conformance client' },
      });
      assert.equal(result.protocolVersion, PROTOCOL_VERSION);
    },
  },
  {
    id: 'transport/initialize/unknown-optional-field',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization ignores unknown optional parameter fields.',
    spec: '/specification/versioning#compatibility-guarantee',
    async run(t) {
      const connection = await t.connect();
      const result = await initialize(t, connection, { [t.uniqueId('future-field')]: { enabled: true } });
      assert.equal(result.protocolVersion, PROTOCOL_VERSION);
    },
  },
  {
    id: 'transport/initialize/unknown-metadata',
    area: 'Transport & lifecycle',
    feature: 'initialize',
    title: 'Initialization ignores unknown request metadata keys.',
    spec: '/reference/common#baseparams',
    async run(t) {
      const connection = await t.connect();
      const result = await initialize(t, connection, {
        _meta: { [`conformance.example/${t.uniqueId('extension')}`]: { text: 'ignored', nested: [null, false, 3] } },
      });
      assert.equal(result.protocolVersion, PROTOCOL_VERSION);
    },
  },
  ...versionTests,
];
