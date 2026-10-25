import type { ResolvedConfig } from '../config/schema.ts';
import { configuredQuerySecrets } from '../config/secrets.ts';

export function configuredSecrets(config: ResolvedConfig): string[] {
  const url = new URL(config.target.url);
  return [...Object.values(config.target.headers), ...config.auth.tokens.map(({ token }) => token),
    ...configuredQuerySecrets(config), decodeURIComponent(url.username), decodeURIComponent(url.password)]
    .filter((secret) => secret.length >= 6).flatMap((secret) => [secret, encodeURIComponent(secret)]).sort((a, b) => b.length - a.length);
}

export function redact(value: unknown, secrets: readonly string[]): unknown {
  const text = (input: string) => secrets.reduce((result, secret) => secret.length >= 6 ? result.replaceAll(secret, '[REDACTED]') : result, input);
  const visit = (input: unknown): unknown => {
    if (typeof input === 'string') return text(input);
    if (Array.isArray(input)) return input.map(visit);
    if (input && typeof input === 'object') {
      const object = input as Record<string, unknown>;
      const result = Object.fromEntries(Object.entries(object).map(([key, entry]) => [text(key), visit(entry)]));
      if (object.method === 'authenticate' && result.params && typeof result.params === 'object') {
        (result.params as Record<string, unknown>).token = '[REDACTED]';
      }
      return result;
    }
    return input;
  };
  return visit(value);
}

export function redactEndpoint(endpoint: string): string {
  const url = new URL(endpoint);
  if (url.username) url.username = 'REDACTED';
  if (url.password) url.password = 'REDACTED';
  for (const name of new Set(url.searchParams.keys())) url.searchParams.set(name, 'REDACTED');
  return url.href;
}

export function redactFrame(text: string, secrets: readonly string[]): unknown {
  try { return redact(JSON.parse(text), secrets); }
  catch {
    return (redact(text, secrets) as string).replace(/("token"\s*:\s*)"(?:[^"\\]|\\.)*"/g, '$1"[REDACTED]"');
  }
}
