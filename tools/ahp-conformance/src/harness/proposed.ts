import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import { RpcError } from './types.ts';
import type { Client, ClientOptions, TestContext } from './types.ts';

/**
 * Proposed protocol version: AHP `main` after the 1.0.0 release, pinned to
 * microsoft/agent-host-protocol@63c52b8a. Tests gated with `requires.proposed` cite this revision.
 */
export const PROPOSED_PROTOCOL_VERSION = '1.1.0';

/** Opens a client that negotiates the proposed protocol version, skipping the test if the host can't. */
export async function proposedClient(t: TestContext, options: Omit<ClientOptions, 'protocolVersions'> = {}): Promise<Client> {
  try {
    const client = await t.client({ ...options, protocolVersions: [PROPOSED_PROTOCOL_VERSION] });
    if (client.initializeResult.protocolVersion !== PROPOSED_PROTOCOL_VERSION) {
      t.skip(`Host negotiated ${client.initializeResult.protocolVersion} instead of proposed ${PROPOSED_PROTOCOL_VERSION}`);
    }
    return client;
  } catch (error) {
    if (error instanceof RpcError && error.code === AhpErrorCodes.UnsupportedProtocolVersion) {
      t.skip(`Host does not support proposed protocol ${PROPOSED_PROTOCOL_VERSION}`);
    }
    throw error;
  }
}
