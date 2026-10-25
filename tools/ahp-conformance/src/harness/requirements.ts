import type { Requirement } from './types.ts';

const agentCapability = (label: string, check: Requirement['check']): Requirement => ({ label, kind: 'capability', check });

export const requires = {
  /** A language model is available to run turns (`model.enabled` in the config). */
  model: { label: 'model', kind: 'config', check: (p) => p.config.model.enabled } satisfies Requirement,
  /** The model is the deterministic-openai fixture (`model.deterministic`), for fixture-only prompts. */
  deterministicModel: {
    label: 'model.deterministic',
    kind: 'config',
    check: (p) => p.config.model.enabled && p.config.model.deterministic,
  } satisfies Requirement,
  /** The host sees the runner's scratch directory at the same path (`workspace.sharedFilesystem`). */
  sharedFilesystem: {
    label: 'sharedFilesystem',
    kind: 'config',
    check: (p) => p.config.workspace.sharedFilesystem,
  } satisfies Requirement,
  /** Proposed, not-yet-ratified protocol behavior is opted into (`run.proposed` / `--proposed`). */
  proposed: { label: 'run.proposed', kind: 'config', check: (p) => p.config.run.proposed } satisfies Requirement,
  completionTriggerCharacters: {
    label: 'completionTriggerCharacters', kind: 'capability',
    check: (p) => (p.initialize.completionTriggerCharacters?.length ?? 0) > 0,
  } satisfies Requirement,
  terminalCommandPrefix: {
    label: 'terminalCommandPrefix', kind: 'capability', check: (p) => Boolean(p.initialize.terminalCommandPrefix),
  } satisfies Requirement,
  automations: { label: 'automations', kind: 'capability', check: (p) => p.initialize.automations !== undefined } satisfies Requirement,
  automationCreate: {
    label: 'automations.create', kind: 'capability', check: (p) => p.initialize.automations?.create !== undefined,
  } satisfies Requirement,
  telemetry: {
    label: 'telemetry', kind: 'capability',
    check: (p) => Object.values(p.initialize.telemetry ?? {}).some((uri) => typeof uri === 'string'),
  } satisfies Requirement,
  /** At least one AHP auth token is configured (`auth.tokens`). */
  authTokens: { label: 'auth.tokens', kind: 'config', check: (p) => p.config.auth.tokens.length > 0 } satisfies Requirement,
  protectedResources: agentCapability('agent.protectedResources', (p) => (p.agent.protectedResources?.length ?? 0) > 0),
  multipleChats: agentCapability('multipleChats', (p) => p.agent.capabilities?.multipleChats !== undefined),
  fork: agentCapability('multipleChats.fork', (p) => p.agent.capabilities?.multipleChats?.fork === true),
  sideChat: agentCapability('multipleChats.sideChat', (p) => p.agent.capabilities?.multipleChats?.sideChat === true),
  multipleWorkingDirectories: agentCapability(
    'multipleWorkingDirectories',
    (p) => p.agent.capabilities?.multipleWorkingDirectories !== undefined,
  ),
} as const;
