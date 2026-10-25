import { MessageKind } from '@microsoft/agent-host-protocol';
import type { ActionEnvelope } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import { requires } from '../../harness/requirements.ts';
import { RpcError } from '../../harness/types.ts';
import type { LooseStateAction, TestDefinition } from '../../harness/types.ts';
import {
  CATALOGUE, accepted, afterActions, assertEntry, catalogue, createAutomation, dispatchOutcome, manualDefinition,
  ownAutomation, record, requireOperation, runtimeDisabled,
} from './helpers.ts';

export const tests: TestDefinition[] = [
  {
    id: 'automations/subscribe/catalogue-snapshot',
    area: 'Automations',
    feature: 'subscribe',
    title: 'An advertised automation catalogue returns a complete state snapshot.',
    spec: '/specification/automation-channel#subscription-and-reconnect',
    requires: [requires.automations],
    async run(t) {
      const snapshot = await catalogue(await t.client());
      for (const entry of snapshot.state.entries) assertEntry(entry);
    },
  },
  {
    id: 'automations/create/authoritative-set',
    area: 'Automations',
    feature: 'automation/createRequested',
    title: 'An accepted create publishes a full authoritative automation entry.',
    spec: '/specification/automation-channel#actions',
    requires: [requires.automations, requires.automationCreate],
    async run(t) {
      const client = await t.client();
      await catalogue(client);
      const { resource, entry, definition } = await createAutomation(t, client);
      assertEntry(entry);
      assert.equal(entry.resource, resource);
      assert.equal(entry.definition.title, definition.title);
      assert.equal(entry.definition.message.text, definition.message.text);
      assert.equal(entry.definition.message.origin.kind, definition.message.origin.kind);
      assert.equal(entry.definition.enabled, false);
      assert.deepEqual(entry.definition.triggers, []);
      const saved = (await catalogue(client)).state.entries.find((value) => value.resource === resource);
      assert.deepEqual(saved, entry);
    },
  },
  {
    id: 'automations/update/disjoint-patches-compose',
    area: 'Automations',
    feature: 'automation/updateRequested',
    title: 'Accepted disjoint automation patches compose without overwriting each other.',
    spec: '/specification/automation-channel#actions',
    requires: [requires.automations, requires.automationCreate],
    async run(t) {
      const client = await t.client();
      await catalogue(client);
      const owned = await createAutomation(t, client);
      await requireOperation(t, client, owned.resource, 'update');
      const other = await t.client();
      await catalogue(other);
      const title = t.uniqueId('updated-manual');
      const metadata = { conformance: t.uniqueId('patch') };
      await Promise.all([
        accepted(client, { type: 'automation/updateRequested', resource: owned.resource, changes: { title } }),
        accepted(other, { type: 'automation/updateRequested', resource: owned.resource, changes: { _meta: metadata } }),
      ]);
      await client.waitForAction(CATALOGUE, (event) => event.action.type === 'automation/set' &&
        event.action.automation.resource === owned.resource && event.action.automation.definition.title === title &&
        event.action.automation.definition._meta?.conformance === metadata.conformance);
      const entry = (await catalogue(client)).state.entries.find((value) => value.resource === owned.resource);
      assert.equal(entry?.definition.title, title);
      assert.deepEqual(entry?.definition._meta, metadata);
    },
  },
  {
    id: 'automations/validation/duplicate-disable-kind-rejected',
    area: 'Automations',
    feature: 'disableConditions',
    title: 'Create and update reject duplicate disable-condition kinds without changing catalogue state.',
    spec: '/guide/automations#disable-conditions',
    requires: [requires.automations, requires.automationCreate],
    async run(t) {
      const client = await t.client();
      await catalogue(client);
      const invalid = ownAutomation(t, client);
      const definition = manualDefinition(t);
      const disableConditions = [
        { kind: 'afterRuns' as const, max: 1 },
        { kind: 'afterRuns' as const, max: 1 },
      ] as NonNullable<typeof definition.disableConditions>;
      const rejected = await dispatchOutcome(client, {
        type: 'automation/createRequested', resource: invalid.resource, definition: { ...definition, disableConditions },
      });
      assert.ok(rejected.rejectionReason, 'duplicate condition kinds must reject creation');
      assert.ok(!(await catalogue(client)).state.entries.some((entry) => entry.resource === invalid.resource));
      const owned = await createAutomation(t, client);
      const before = await requireOperation(t, client, owned.resource, 'update');
      const update = await dispatchOutcome(client, {
        type: 'automation/updateRequested', resource: owned.resource, changes: { disableConditions },
      });
      assert.ok(update.rejectionReason, 'duplicate condition kinds must reject updates');
      const after = (await catalogue(client)).state.entries.find((entry) => entry.resource === owned.resource);
      assert.deepEqual(after?.definition, before.definition);
      assert.equal(after?.runCount, before.runCount);
    },
  },
  {
    id: 'automations/remove/committed-before-echo',
    area: 'Automations',
    feature: 'automation/removed',
    title: 'An accepted removal is absent from a fresh catalogue after its echo.',
    spec: '/specification/automation-channel#actions',
    requires: [requires.automations, requires.automationCreate],
    async run(t) {
      const client = await t.client();
      await catalogue(client);
      const owned = await createAutomation(t, client);
      await accepted(client, { type: 'automation/removed', resource: owned.resource });
      const observer = await t.client();
      assert.ok(!(await catalogue(observer)).state.entries.some((entry) => entry.resource === owned.resource));
    },
  },
  {
    id: 'automations/remove/unknown-resource-noop',
    area: 'Automations',
    feature: 'automation/removed',
    title: 'Removing an unknown automation is an accepted catalogue no-op.',
    spec: '/specification/automation-channel#actions',
    requires: [requires.automations],
    async run(t) {
      const client = await t.client();
      const before = await catalogue(client);
      const resource = `ahp-automation:/${t.uniqueId('never-created')}`;
      assert.ok(!before.state.entries.some((entry) => entry.resource === resource));
      client.dispatch(CATALOGUE, { type: 'automation/removed', resource });
      await client.expectNoNotification<ActionEnvelope>('action', (event) =>
        event.channel === CATALOGUE && event.action.type === 'automation/set' && event.action.automation.resource === resource, 400);
      const rejected = client.actions(CATALOGUE).find((event) => event.serverSeq > before.fromSeq &&
        event.action.type === 'automation/removed' && event.action.resource === resource && event.rejectionReason !== undefined);
      assert.equal(rejected, undefined, 'removing an unknown resource must be a no-op, not a rejection');
      const after = await catalogue(client);
      assert.deepEqual(after.state.entries, afterActions(client, before, after.fromSeq).entries);
      assert.ok(!after.state.entries.some((entry) => entry.resource === resource));
    },
  },
  {
    id: 'automations/triggers/discovery-command',
    area: 'Automations',
    feature: 'listAutomationTriggerDefinitions',
    title: 'Trigger discovery returns event authoring metadata from the root channel.',
    spec: '/guide/automations#event-triggers',
    requires: [requires.automations],
    async run(t) {
      const client = await t.client();
      let result: unknown;
      try {
        result = await client.request('listAutomationTriggerDefinitions', {
          channel: 'ahp-root://', provider: t.profile.agent.provider,
        });
      } catch (error) {
        if (error instanceof RpcError && runtimeDisabled(client) && /automations are disabled/i.test(error.message)) {
          return t.skip('The advertised root automationsEnabled setting disables the automation runtime (including trigger discovery).');
        }
        throw error;
      }
      record(result);
      assert.ok(Array.isArray(result.items));
      for (const item of result.items) {
        record(item);
        assert.equal(typeof item.type, 'string');
        assert.equal(typeof item.title, 'string');
        assert.ok(Array.isArray(item.events));
        for (const event of item.events) {
          record(event);
          assert.equal(typeof event.id, 'string');
          assert.equal(typeof event.title, 'string');
        }
        if (item.configSchema !== undefined) record(item.configSchema);
      }
    },
  },
  {
    id: 'automations/actions/authoritative-rejection',
    area: 'Automations',
    feature: 'automation/updateRequested',
    title: 'An invalid automation definition update is rejected without authoritative mutation.',
    spec: '/specification/automation-channel#actions',
    requires: [requires.automations, requires.automationCreate],
    async run(t) {
      const client = await t.client();
      await catalogue(client);
      const owned = await createAutomation(t, client);
      const before = await requireOperation(t, client, owned.resource, 'update');
      const action: LooseStateAction = {
        type: 'automation/updateRequested', resource: owned.resource,
        changes: { message: { text: 'ok', origin: { kind: MessageKind.User } } },
      };
      const rejected = await dispatchOutcome(client, action);
      assert.ok(rejected.rejectionReason, 'an automation message with user origin must be rejected');
      const after = (await catalogue(client)).state.entries.find((entry) => entry.resource === owned.resource);
      assert.deepEqual(after?.definition, before.definition);
    },
  },
];
