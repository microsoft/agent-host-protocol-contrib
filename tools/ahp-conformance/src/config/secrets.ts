import { exec } from 'node:child_process';
import { emitKeypressEvents } from 'node:readline';
import { promisify } from 'node:util';
import type { ResolvedConfig, SecretSource } from './schema.ts';

const execute = promisify(exec);

export async function promptSecret(label: string): Promise<string> {
  if (!process.stdin.isTTY || !process.stdout.isTTY) {
    throw new Error(`Secret prompt "${label}" requires an interactive TTY; use an env or command source instead`);
  }
  return new Promise((resolve, reject) => {
    const input = process.stdin;
    const wasRaw = input.isRaw;
    const wasPaused = input.isPaused();
    let value = '';
    emitKeypressEvents(input);
    input.setRawMode(true);
    input.resume();
    process.stdout.write(`${label}: `);
    const finish = (error?: Error) => {
      input.off('keypress', onKey);
      input.setRawMode(wasRaw);
      if (wasPaused) input.pause();
      process.stdout.write('\n');
      if (error) reject(error);
      else resolve(value);
    };
    const onKey = (text: string | undefined, key: { name?: string; ctrl?: boolean }) => {
      if (key.ctrl && key.name === 'c') finish(new Error('Secret prompt cancelled'));
      else if (key.name === 'return' || key.name === 'enter') finish();
      else if (key.name === 'backspace') value = [...value].slice(0, -1).join('');
      else if (text && !key.ctrl && !text.startsWith('\u001b')) value += text;
    };
    input.on('keypress', onKey);
  });
}

export async function resolveSecret(source: SecretSource, cwd = process.cwd()): Promise<string> {
  if (typeof source === 'string') return source;
  if ('value' in source) return source.value;
  if ('env' in source) {
    const value = process.env[source.env];
    if (value === undefined) throw new Error(`Environment variable ${source.env} is not set`);
    return value;
  }
  if ('prompt' in source) return promptSecret(source.prompt);
  try {
    const { stdout } = await execute(source.command, { cwd, timeout: 30_000, maxBuffer: 1024 * 1024 });
    return stdout.trim();
  } catch {
    throw new Error('Secret command failed (check the command in your configuration)');
  }
}


const querySecrets = new WeakMap<ResolvedConfig, readonly string[]>();

export function rememberQuerySecrets(config: ResolvedConfig, values: readonly string[]): void {
  querySecrets.set(config, values);
}

export function configuredQuerySecrets(config: ResolvedConfig): readonly string[] {
  return querySecrets.get(config) ?? [];
}
