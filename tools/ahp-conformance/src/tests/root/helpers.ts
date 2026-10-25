import type { RootState } from '@microsoft/agent-host-protocol';
import { assert } from '../../harness/assert.ts';
import type { Client, TestContext } from '../../harness/types.ts';

export const ROOT = 'ahp-root://';

export function assertRecord(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

export function rootState(client: Client): RootState {
  const state = client.state<RootState>(ROOT);
  assert.ok(state, 'a root subscriber must have a root state');
  return state;
}

export async function renameForSetup(t: TestContext, client: Client, uri: string, title: string): Promise<void> {
  const clientSeq = client.dispatch(uri, { type: 'session/titleChanged', title });
  const echo = await client.waitForAction(uri, (event) =>
    event.origin?.clientId === client.clientId && event.origin.clientSeq === clientSeq);
  if (echo.rejectionReason !== undefined) {
    t.skip('The setup rename was rejected; session-title acceptance is covered by sessions/session-actions/title-changed.');
  }
}

export async function observe(client: Client): Promise<void> {
  await client.expectNoNotification('action', () => false, 400);
}
