import { existsSync } from 'node:fs';
import { join } from 'node:path';

/**
 * Locates a Dojo helper executable. An explicit `DOJO_<NAME>_EXE` override wins,
 * then the copy installed under `$XDG_DATA_HOME/dojo/bin`, then release and debug
 * builds in the checkout so development keeps working without installation.
 */
export function helperBinary(root, name, { env = process.env, exists = existsSync } = {}) {
  const variable = `DOJO_${name.replace(/^dojo-/, '').toUpperCase().replace(/-/g, '_')}_EXE`;
  if (env[variable]) return env[variable];
  const dataHome = env.XDG_DATA_HOME || join(env.HOME ?? '', '.local/share');
  const candidates = [join(dataHome, 'dojo/bin', name), join(root, 'target/release', name), join(root, 'target/debug', name)];
  return candidates.find(candidate => exists(candidate)) ?? candidates.at(-1);
}
