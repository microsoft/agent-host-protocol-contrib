import { readFile } from 'node:fs/promises';
import { dirname, resolve } from 'node:path';
import { parse } from 'yaml';
import { configSchema } from './schema.ts';
import type { ResolvedConfig, SecretSource } from './schema.ts';
import { rememberQuerySecrets, resolveSecret } from './secrets.ts';
import { createScratchDir } from './workspace.ts';

export interface ConfigOverrides {
  reporter?: 'cli' | 'html';
  open?: boolean;
  jsonFile?: string;
  proposed?: boolean;
}

export async function resolveConfig(input: unknown, configPath: string, overrides: ConfigOverrides = {}): Promise<ResolvedConfig> {
  const parsed = configSchema.safeParse(input);
  if (!parsed.success) {
    throw new Error(`Invalid configuration:\n${parsed.error.issues.map((issue) => `  ${issue.path.join('.') || 'config'}: ${issue.message}`).join('\n')}`);
  }
  const value = parsed.data;
  const absolutePath = resolve(configPath);
  const dir = dirname(absolutePath);
  const resolveMap = async (map: Record<string, SecretSource>) => {
    const result: Record<string, string> = {};
    for (const [key, source] of Object.entries(map)) result[key] = await resolveSecret(source, dir);
    return result;
  };
  const url = new URL(value.target.url);
  const query = await resolveMap(value.target.query);
  for (const [key, secret] of Object.entries(query)) url.searchParams.set(key, secret);
  const headers = await resolveMap(value.target.headers);
  const tokens = [];
  for (const { resource, token } of value.auth.tokens) tokens.push({ resource, token: await resolveSecret(token, dir) });
  const scratchDir = await createScratchDir(value.workspace.scratchDir, dir);
  const substitute = (text: string) => text.replaceAll('${scratchDir}', scratchDir);
  const reporter = overrides.reporter ?? value.report.reporter;
  const config: ResolvedConfig = {
    configPath: absolutePath,
    target: { url: url.href, headers },
    ...(value.host && { host: {
      command: substitute(value.host.command),
      cwd: resolve(dir, value.host.cwd),
      env: Object.fromEntries(Object.entries(value.host.env).map(([key, entry]) => [key, substitute(entry)])),
      readyTimeoutMs: value.host.readyTimeoutMs,
    } }),
    auth: { tokens },
    agent: value.agent,
    model: value.model,
    quirks: value.quirks,
    workspace: { ...value.workspace, scratchDir },
    run: { ...value.run, proposed: overrides.proposed ?? value.run.proposed },
    report: {
      reporter: reporter === 'auto' ? (process.stdin.isTTY && process.stdout.isTTY && process.env.CI === undefined ? 'html' : 'cli') : reporter,
      open: overrides.open ?? value.report.open,
      htmlFile: resolve(dir, value.report.htmlFile),
      ...((overrides.jsonFile ?? value.report.jsonFile) && { jsonFile: overrides.jsonFile ? resolve(overrides.jsonFile) : resolve(dir, value.report.jsonFile!) }),
    },
  };
  rememberQuerySecrets(config, Object.values(query));
  return config;
}

export async function loadConfig(path: string, overrides: ConfigOverrides = {}): Promise<ResolvedConfig> {
  let input: unknown;
  try { input = parse(await readFile(path, 'utf8')); }
  catch (error) { throw new Error(`Cannot read configuration ${path}: ${error instanceof Error ? error.message : String(error)}`); }
  return resolveConfig(input, path, overrides);
}
