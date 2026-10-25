import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import type { Requirement, TestDefinition } from '../../harness/types.ts';
import { CATALOGUE, catalogue, createAutomation, record, requireOperation } from './helpers.ts';

const cancellableRuns: Requirement = {
  label: 'automations.runCancellation (safe run cleanup)',
  kind: 'capability',
  check: (profile) => profile.initialize.automations?.runCancellation !== undefined,
};

export const tests: TestDefinition[] = [
  {
    id: 'automations/run/retry-request-id',
    area: 'Automations',
    feature: 'runAutomation',
    title: 'Retrying runAutomation after reconnect returns the original run URI.',
    spec: '/specification/automation-run-channel#reconciliation',
    requires: [requires.automations, requires.automationCreate, requires.model, cancellableRuns],
    async run(t) {
      const owner = await t.client();
      await catalogue(owner);
      const owned = await createAutomation(t, owner, { enabled: true });
      await requireOperation(t, owner, owned.resource, 'run');
      const runner = await t.client();
      await catalogue(runner);
      const params = { channel: CATALOGUE, automation: owned.resource, requestId: t.uniqueId('manual-run') };
      owned.mayHaveRuns = true;
      try {
        const first = await runner.request('runAutomation', params);
        record(first);
        assert.equal(typeof first.resource, 'string');
        assert.equal(new URL(first.resource as string).protocol, 'ahp-automation-run:');
        owned.runs.add(first.resource as string);
        const lastSeenServerSeq = runner.lastServerSeq;
        await runner.close();
        const resumed = await t.connect({ label: 'automation-run-retry' });
        await resumed.request('reconnect', {
          channel: 'ahp-root://', clientId: runner.clientId, lastSeenServerSeq, subscriptions: [CATALOGUE],
        });
        for (const token of t.config.auth.tokens) await resumed.request('authenticate', { channel: 'ahp-root://', ...token });
        const second = await resumed.request('runAutomation', params);
        record(second);
        if (typeof second.resource === 'string' && second.resource.startsWith('ahp-automation-run:')) owned.runs.add(second.resource);
        assert.equal(second.resource, first.resource, 'the durable requestId must identify the original run');
      } finally {
        await owned.cleanup();
      }
    },
  },
];
