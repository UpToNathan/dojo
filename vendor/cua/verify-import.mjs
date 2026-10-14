#!/usr/bin/env node
// Dojo import tooling. No upstream code or runtime configuration is executed.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, mkdirSync, existsSync, readdirSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const ledger = JSON.parse(readFileSync(resolve(root, 'vendor/cua/ledger.json')));
const args = process.argv.slice(2);
let source, output;
for (let i = 0; i < args.length; i++) {
  if (args[i] === '--source' && args[i + 1]) source = resolve(args[++i]);
  else if (args[i] === '--extract' && args[i + 1]) output = resolve(args[++i]);
  else throw new Error('Usage: node vendor/cua/verify-import.mjs [--source CHECKOUT] [--extract EMPTY_OUTPUT]');
}
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
function safePath(base, path) {
  const dest = resolve(base, path);
  const rel = relative(base, dest);
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) throw new Error(`Unsafe path: ${path}`);
  return dest;
}
function git(...args) {
  return execFileSync('git', ['-C', source, ...args], { maxBuffer: 32 * 1024 * 1024 });
}
if (source && git('rev-parse', 'HEAD').toString().trim() !== ledger.upstream.revision) {
  throw new Error('Source checkout HEAD does not match the pinned revision');
}
if (output) {
  if (!source) throw new Error('--extract requires --source');
  if (existsSync(output) && readdirSync(output).length) throw new Error('Extraction output must be empty');
  mkdirSync(output, { recursive: true });
}
for (const file of ledger.files) {
  const maintained = readFileSync(safePath(root, file.path));
  if (sha(maintained) !== file.sha256) throw new Error(`Local byte mismatch: ${file.path}`);
  let upstream;
  if (source && file.upstream_path) {
    // Read committed objects, never trust a dirty source working tree.
    upstream = git('show', `${ledger.upstream.revision}:${file.upstream_path}`);
    if (sha(upstream) !== file.upstream_sha256) throw new Error(`Upstream mismatch: ${file.upstream_path}`);
    if (file.state === 'copied' && sha(upstream) !== file.sha256) throw new Error(`Copy differs: ${file.path}`);
  }
  if (file.patch) {
    const patch = readFileSync(safePath(root, file.patch));
    if (sha(patch) !== file.patch_sha256) throw new Error(`Patch mismatch: ${file.patch}`);
  }
  if (output) {
    const dest = safePath(output, file.path);
    mkdirSync(dirname(dest), { recursive: true });
    // Modified metadata and local glue/locks are maintained inputs, not generated
    // from an entire upstream workspace. Their checksums are verified above.
    writeFileSync(dest, file.state === 'copied' ? upstream : maintained);
  }
}
console.log(`Verified ${ledger.files.length} maintained files${source ? ' and pinned upstream objects' : ' offline'}${output ? `; extracted to ${output}` : ''}.`);
