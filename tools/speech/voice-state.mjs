import { readFile, mkdir, writeFile, rename } from 'node:fs/promises';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

export const dataDir = join(process.env.XDG_STATE_HOME || join(homedir(), '.local/state'), 'dojo/voice');
export async function saveJson(path, value, serialized = JSON.stringify(value)) {
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  const temporary = `${path}.${randomUUID()}.tmp`;
  await writeFile(temporary, serialized, { mode: 0o600 });
  await rename(temporary, path);
}
export async function readJson(path, fallback) {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return fallback; throw error; }
}

