import type { ConfigPropertySchema, ResolveSessionConfigResult } from '@microsoft/agent-host-protocol';
import type { TestContext, TestDefinition } from '../../harness/types.ts';
import { ROOT } from './helpers.ts';

function fillRequired(
  t: TestContext,
  properties: Record<string, ConfigPropertySchema>,
  required: string[],
  current: Record<string, unknown>,
): Record<string, unknown> {
  const values = structuredClone(current);
  for (const property of required) {
    const schema = properties[property];
    if (!schema) t.skip(`Required session config property ${property} has no advertised descriptor.`);
    if (!Object.hasOwn(values, property)) {
      if (schema.default !== undefined) values[property] = structuredClone(schema.default);
      else if (schema.enum?.length) values[property] = schema.enum[0];
      else if (schema.type === 'object') values[property] = {};
      else t.skip(`Required session config property ${property} has no safely selectable default or enum value.`);
    }
  }
  for (const [property, value] of Object.entries(values)) {
    const schema = properties[property];
    if (schema?.type === 'object' && value !== null && typeof value === 'object' && !Array.isArray(value)) {
      values[property] = fillRequired(t, schema.properties ?? {}, schema.required ?? [], value as Record<string, unknown>);
    }
  }
  return values;
}

export const tests: TestDefinition[] = [
  {
    id: 'sessions/resolve-session-config/create-round-trip',
    area: 'Root & sessions', feature: 'resolveSessionConfig',
    title: 'Resolved session configuration values can be passed unchanged to session creation.',
    spec: '/reference/root#resolvesessionconfig',
    async run(t) {
      const client = await t.client();
      const context = { channel: ROOT, provider: t.profile.agent.provider, workingDirectory: t.scratchUri() };
      let result = await client.request<ResolveSessionConfigResult>('resolveSessionConfig', context);
      for (let attempt = 0; attempt < 3; attempt++) {
        const selected = fillRequired(t, result.schema.properties, result.schema.required ?? [], result.values);
        if (JSON.stringify(selected) === JSON.stringify(result.values)) break;
        result = await client.request<ResolveSessionConfigResult>('resolveSessionConfig', { ...context, config: selected });
      }
      const remaining = fillRequired(t, result.schema.properties, result.schema.required ?? [], result.values);
      if (JSON.stringify(remaining) !== JSON.stringify(result.values)) {
        t.skip('The contextual session schema still requires additional selections after three resolution rounds.');
      }
      const session = await t.session(client, { config: result.values });
      await session.ready();
    },
  },
];
