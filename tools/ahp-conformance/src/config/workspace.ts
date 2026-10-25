import { existsSync, rmSync } from 'node:fs';
import { mkdir, mkdtemp, realpath, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

const ownedDirectories = new Set<string>();

/** Returns the enclosing git work tree of `path`, if any. */
export function findGitWorkTree(path: string): string | undefined {
  for (let current = path; ; current = dirname(current)) {
    if (existsSync(join(current, '.git'))) return current;
    if (dirname(current) === current) return undefined;
  }
}

export async function createScratchDir(configuredPath: string | undefined, configDir: string): Promise<string> {
  const parent = configuredPath === undefined ? tmpdir() : resolve(configDir, configuredPath);
  // Hosts may isolate sessions in git repositories by creating worktrees and branches there.
  const repository = findGitWorkTree(parent);
  if (repository) {
    throw new Error(`Scratch directory ${parent} is inside the git repository ${repository}; agent hosts may create worktrees or branches there. Choose a scratch directory outside any repository.`);
  }
  await mkdir(parent, { recursive: true });
  const prefix = configuredPath === undefined ? 'ahp-conformance' : 'run';
  const scratchDir = await realpath(await mkdtemp(join(parent, `${prefix}-${Date.now()}-`)));
  ownedDirectories.add(scratchDir);
  return scratchDir;
}

export async function removeScratchDir(scratchDir: string): Promise<void> {
  if (!ownedDirectories.has(scratchDir)) return;
  await rm(scratchDir, { recursive: true, force: true });
  ownedDirectories.delete(scratchDir);
}


export function removeScratchDirSync(scratchDir: string): void {
  if (!ownedDirectories.has(scratchDir)) return;
  rmSync(scratchDir, { recursive: true, force: true });
  ownedDirectories.delete(scratchDir);
}
