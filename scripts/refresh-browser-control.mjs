#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, readdirSync, mkdtempSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';

const root = resolve(import.meta.dirname, '..');
const revision = 'b4e4238742fa73daabfb664f1108113e9577849d';
const source = process.argv[process.argv.indexOf('--source') + 1];
if (!process.argv.includes('--source') || !source) throw new Error('Pass --source with the pinned upstream Git checkout');
const git = (args, options = {}) => execFileSync('git', ['-C', resolve(source), ...args], options);
if (git(['rev-parse', 'HEAD']).toString().trim() !== revision) throw new Error('Upstream checkout does not match the pinned revision');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const temporary = mkdtempSync(join(tmpdir(), 'dojo-browser-provenance-'));
const fork = join(root, 'vendor/browser-control/upstream');
const env = { ...process.env, GIT_INDEX_FILE: join(temporary, 'index') };
const excluded = new Set(['node_modules', 'dist', '.git', '.pnpm-store', 'target', 'test-results', 'playwright-report']);
const walk = (directory, prefix = '') => readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
  if (excluded.has(entry.name)) return [];
  if (entry.isSymbolicLink()) throw new Error(`Unexpected source symlink: ${prefix}${entry.name}`);
  const relative = prefix + entry.name;
  return entry.isDirectory() ? walk(join(directory, entry.name), relative + '/') : [relative];
}).sort();
try {
  git(['read-tree', revision], { env });
  const upstreamFiles = new Set(git(['ls-tree', '-r', '--name-only', revision]).toString().trim().split('\n'));
  const paths = [...new Set([...walk(fork), ...upstreamFiles])].sort();
  // Use a private index so neither checkout's real index or worktree is changed.
  git(['--work-tree=' + fork, 'add', '-A', '--', ...paths], { env });
  const patch = git(['--work-tree=' + fork, 'diff', '--cached', '--binary', '--no-renames', revision], { env, maxBuffer: 16 * 1024 * 1024 });
  const files = walk(fork).map(path => {
    const bytes = readFileSync(join(fork, path));
    let original;
    original = upstreamFiles.has(path) ? git(['show', revision + ':' + path]) : undefined;
    return { path, sha256: hash(bytes), upstreamSha256: original === undefined ? null : hash(original), state: original === undefined ? 'added' : bytes.equals(original) ? 'copied' : 'modified' };
  });
  const native = walk(join(root, 'crates/dojo-browser-control')).map(path => ({ path, sha256: hash(readFileSync(join(root, 'crates/dojo-browser-control', path))) }));
  const niri = ['vendor/niri/src/browser_control.rs', 'vendor/niri/src/niri.rs', 'vendor/niri/src/lib.rs', 'vendor/niri/src/agent_input/shared.rs', 'vendor/niri/src/input/mod.rs', 'vendor/niri/src/ipc/client.rs'].map(path => ({ path, sha256: hash(readFileSync(join(root, path))) }));
  const manifest = { niri, repository: 'https://github.com/anomalyco/browser-control', revision, version: '0.8.4', pnpmVersion: '11.20.0', extensionVersion: '0.0.25', patchSha256: hash(patch), lockSha256: hash(readFileSync(join(fork, 'pnpm-lock.yaml'))), files, native };
  manifest.sourceIdentity = hash(Buffer.from(JSON.stringify(manifest)));
  const outputs = [[join(root, 'vendor/browser-control/dojo.patch'), patch], [join(root, 'vendor/browser-control/manifest.json'), Buffer.from(JSON.stringify(manifest, null, 2) + '\n')]];
  if (process.argv.includes('--write')) for (const [path, bytes] of outputs) writeFileSync(path, bytes);
  else for (const [path, bytes] of outputs) if (!readFileSync(path).equals(bytes)) throw new Error(`${path} differs; regenerate after reviewing the source changes`);
  console.log(`Browser Control ${revision}: ${files.length} source files, ${files.filter(file => file.state !== 'copied').length} local changes, identity ${manifest.sourceIdentity}`);
} finally { rmSync(temporary, { recursive: true, force: true }); }
