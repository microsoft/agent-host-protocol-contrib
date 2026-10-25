import type { HostProfile, TestDefinition } from '../harness/types.ts';

export function skipReason(test: Pick<TestDefinition, 'requires'>, profile: HostProfile): string | undefined {
  const requirement = test.requires?.find((entry) => !entry.check(profile));
  if (!requirement) return undefined;
  return requirement.kind === 'capability' ? `Host does not advertise ${requirement.label}` : `Disabled in config: ${requirement.label}`;
}
