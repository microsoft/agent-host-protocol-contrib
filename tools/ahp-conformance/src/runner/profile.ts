import type { RootState } from '@microsoft/agent-host-protocol';
import type { ResolvedConfig } from '../config/schema.ts';
import { ProtocolClient } from '../harness/client.ts';
import { redactEndpoint } from '../harness/redact.ts';
import type { HostProfile, ProfileSummary, RunEvent } from '../harness/types.ts';

export async function probe(config: ResolvedConfig, signal: AbortSignal, emit: (event: RunEvent) => void): Promise<HostProfile> {
  const client = new ProtocolClient({ config, signal, id: '$probe', label: 'profile', emit });
  try {
    await client.open();
    await client.initialize();
    const selectAgent = (root: RootState) =>
      config.agent.provider ? root.agents.find((entry) => entry.provider === config.agent.provider) : root.agents[0];
    const selectModel = (agent: RootState['agents'][number]) =>
      config.agent.model ? agent.models.find((entry) => entry.id === config.agent.model) : agent.models[0];
    const initial = client.state<RootState>('ahp-root://');
    if (!initial || !Array.isArray(initial.agents)) throw new Error('Host did not return a root snapshot with an agents list');
    if (!selectAgent(initial)) throw new Error(config.agent.provider ? `Host does not advertise agent provider "${config.agent.provider}"` : 'Host does not advertise any agents');
    // Some hosts populate model catalogs asynchronously, e.g. only after authentication.
    const root = config.model.enabled
      ? await client.waitForState<RootState>('ahp-root://', (state) => {
        const agent = selectAgent(state);
        return agent !== undefined && selectModel(agent) !== undefined;
      }, { timeoutMs: config.agent.catalogTimeoutMs }).catch(() => client.state<RootState>('ahp-root://')!)
      : initial;
    const agent = selectAgent(root)!;
    const model = selectModel(agent);
    if (config.agent.model && !model) throw new Error(`Agent "${agent.provider}" did not advertise model "${config.agent.model}" within ${config.agent.catalogTimeoutMs} ms`);
    return { initialize: client.initializeResult, root, agent, model, config };
  } finally { await client.close(); }
}

export function summarizeProfile(profile: HostProfile): ProfileSummary {
  return {
    endpoint: redactEndpoint(profile.config.target.url), protocolVersion: profile.initialize.protocolVersion,
    serverInfo: profile.initialize.serverInfo,
    agent: { provider: profile.agent.provider, displayName: profile.agent.displayName, capabilities: profile.agent.capabilities },
    model: profile.model?.id, scratchDir: profile.config.workspace.scratchDir,
  };
}
