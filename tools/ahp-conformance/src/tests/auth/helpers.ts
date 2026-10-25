import { AhpErrorCodes } from '@microsoft/agent-host-protocol';
import type { AgentInfo, CreateSessionParams, ProtectedResourceMetadata, RootState } from '@microsoft/agent-host-protocol';
import type { ResolvedConfig } from '../../config/schema.ts';
import { assert } from '../../harness/assert.ts';
import { RpcError } from '../../harness/types.ts';
import type { Client, TestContext } from '../../harness/types.ts';

type AuthToken = ResolvedConfig['auth']['tokens'][number];

export function assertRecord(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value !== null && typeof value === 'object' && !Array.isArray(value), 'expected an object');
}

function assertString(value: unknown): asserts value is string {
  assert.equal(typeof value, 'string');
}

export function assertUrl(value: unknown): URL {
  assertString(value);
  return new URL(value);
}

function assertStringArray(value: unknown): asserts value is string[] {
  assert.ok(Array.isArray(value), 'expected an array');
  for (const entry of value) assertString(entry);
}

export const metadataRules: {
  field: keyof ProtectedResourceMetadata;
  id: string;
  title: string;
  check(value: unknown): void;
}[] = [
  {
    field: 'resource_name', id: 'resource-name-string',
    title: 'Protected resource names are strings when supplied.', check: assertString,
  },
  {
    field: 'authorization_servers', id: 'authorization-server-urls',
    title: 'Authorization server identifiers are an array of URLs when supplied.',
    check(value) {
      assertStringArray(value);
      for (const entry of value) assertUrl(entry);
    },
  },
  {
    field: 'jwks_uri', id: 'jwks-uri-url',
    title: 'Protected resource JWK Set locations are URLs when supplied.', check: assertUrl,
  },
  {
    field: 'scopes_supported', id: 'scopes-supported-strings',
    title: 'Supported OAuth scopes are an array of strings when supplied.', check: assertStringArray,
  },
  {
    field: 'bearer_methods_supported', id: 'bearer-methods-supported-strings',
    title: 'Supported bearer presentation methods are an array of strings when supplied.', check: assertStringArray,
  },
  {
    field: 'resource_signing_alg_values_supported', id: 'signing-algorithms-strings',
    title: 'Supported resource signing algorithms are an array of strings when supplied.', check: assertStringArray,
  },
  {
    field: 'resource_documentation', id: 'resource-documentation-url',
    title: 'Protected resource documentation locations are URLs when supplied.', check: assertUrl,
  },
  {
    field: 'resource_policy_uri', id: 'resource-policy-url',
    title: 'Protected resource data-usage policies are URLs when supplied.', check: assertUrl,
  },
  {
    field: 'resource_tos_uri', id: 'resource-terms-url',
    title: 'Protected resource terms of service are URLs when supplied.', check: assertUrl,
  },
  {
    field: 'required', id: 'required-boolean',
    title: 'Protected resource required flags are booleans when supplied.',
    check(value) { assert.equal(typeof value, 'boolean'); },
  },
];

export function assertResourceMetadata(value: unknown): asserts value is ProtectedResourceMetadata {
  assertRecord(value);
  assert.equal(assertUrl(value.resource).protocol, 'https:');
  assertString(value.resource);
  assert.ok(!value.resource.includes('#'), 'resource identifiers must not contain a fragment');
  for (const rule of metadataRules) {
    if (value[rule.field] !== undefined) rule.check(value[rule.field]);
  }
}

export function selectedAgent(t: TestContext, client: Client): AgentInfo {
  const root = client.state<RootState>('ahp-root://');
  assert.ok(root, 'expected an initialized root subscription');
  const agent = root.agents.find((entry) => entry.provider === t.profile.agent.provider);
  assert.ok(agent, 'expected the selected agent in root state');
  return agent;
}

export function protectedResources(t: TestContext, client: Client): ProtectedResourceMetadata[] {
  const resources = selectedAgent(t, client).protectedResources ?? [];
  if (resources.length === 0) t.skip('The selected agent no longer advertises protected resources.');
  return resources;
}

export function requiredResources(t: TestContext, client: Client): ProtectedResourceMetadata[] {
  const resources = protectedResources(t, client).filter((resource) => resource.required !== false);
  if (resources.length === 0) t.skip('The selected agent advertises only optional protected resources.');
  return resources;
}

export function configuredToken(t: TestContext, client: Client): AuthToken {
  const resources = protectedResources(t, client);
  const token = t.config.auth.tokens.find((entry) => resources.some((resource) => resource.resource === entry.resource));
  if (!token) t.skip('No configured token matches a protected resource advertised by the selected agent.');
  if (token.token.length === 0) t.skip('The matching configured token is empty and would revoke authentication.');
  return token;
}

export function requiredTokens(t: TestContext, client: Client): AuthToken[] {
  return requiredResources(t, client).map((resource) => {
    const token = t.config.auth.tokens.find((entry) => entry.resource === resource.resource);
    if (!token) t.skip('A required protected resource has no matching configured token.');
    if (token.token.length === 0) t.skip('A required protected resource has an empty configured token.');
    return token;
  });
}

export function authenticate(client: Client, token: AuthToken): Promise<unknown> {
  return client.request('authenticate', { channel: 'ahp-root://', resource: token.resource, token: token.token });
}

export async function authenticateAll(client: Client, tokens: AuthToken[]): Promise<void> {
  for (const token of tokens) await authenticate(client, token);
}

export function revoke(client: Client, resource: string): Promise<unknown> {
  return client.request('authenticate', { channel: 'ahp-root://', resource, token: '' });
}

export async function observeAuthRequired(
  t: TestContext,
  client: Client,
  overrides?: Partial<CreateSessionParams>,
): Promise<RpcError> {
  try {
    await t.session(client, overrides);
  } catch (error) {
    if (!(error instanceof RpcError)) throw error;
    if (error.code === AhpErrorCodes.AuthRequired) return error;
    t.skip('Session creation did not produce the optional AuthRequired error code.');
  }
  t.skip('The host does not require authentication at session creation.');
}

export function authRequiredResources(error: RpcError): unknown[] {
  assertRecord(error.data);
  assert.ok(Array.isArray(error.data.resources), 'AuthRequired data must contain a resources array');
  return error.data.resources;
}

export async function observeAuthNotification(t: TestContext): Promise<unknown> {
  const client = await t.client({ authenticate: false });
  requiredResources(t, client);
  try {
    await t.session(client);
  } catch (error) {
    if (!(error instanceof RpcError)) throw error;
  }
  try {
    return await client.waitForNotification('auth/required', undefined, { timeoutMs: 400 });
  } catch (error) {
    if (t.signal.aborted) throw error;
    t.skip('The host did not emit an optional auth/required notification within the observation window.');
  }
}
