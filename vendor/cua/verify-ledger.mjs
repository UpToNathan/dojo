#!/usr/bin/env node
// Integrity and exact patch correspondence; no builds, source changes or downloads.
import { createHash } from 'node:crypto';
import { readFileSync, readlinkSync, lstatSync, realpathSync, mkdtempSync, writeFileSync, rmSync, mkdirSync, existsSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync, spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
let root = resolve(dirname(fileURLToPath(import.meta.url)), '../..'), source;
for (let i = 2; i < process.argv.length; i++) {
  if (process.argv[i] === '--source') source = resolve(process.argv[++i]);
  else if (process.argv[i] === '--root') root = resolve(process.argv[++i]);
  else throw new Error('Usage: verify-ledger.mjs [--source PINNED_CHECKOUT] [--root TEST_ROOT]');
}
root = realpathSync(root);
const ledger = JSON.parse(readFileSync(resolve(root, 'vendor/cua/ledger.json')));
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
function safe(path) {
  const file = resolve(root, path), rel = relative(root, file);
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) throw new Error(`Unsafe path: ${path}`);
  if (lstatSync(file).isSymbolicLink() || relative(root, realpathSync(file)).startsWith('..')) throw new Error(`Symlink path refused: ${path}`);
  if (!lstatSync(file).isFile()) throw new Error(`Not a regular file: ${path}`);
  return file;
}
function git(...args) { return execFileSync('git', ['-C', source, ...args], { maxBuffer: 32 * 1024 * 1024 }); }
if (source && git('rev-parse', 'HEAD').toString().trim() !== ledger.upstream.revision) throw new Error('Pinned revision mismatch');
const paths = new Set(), temp = mkdtempSync(join(tmpdir(), 'dojo-verify-ledger-'));
let copies = 0, modifications = 0, wholeImportFiles = 0;
try {
  for (const file of ledger.files) {
    if (paths.has(file.path)) throw new Error(`Duplicate ledger path: ${file.path}`);
    paths.add(file.path);
    if (!['copied','modified','local','translated'].includes(file.state)) throw new Error(`Unknown state: ${file.state}`);
    const maintained = readFileSync(safe(file.path));
    if (sha(maintained) !== file.sha256) throw new Error(`Local byte mismatch: ${file.path}`);
    let original;
    if (file.upstream_path) {
      if (!file.upstream_sha256) throw new Error(`Missing upstream hash: ${file.path}`);
      if (source) {
        original = git('show', `${ledger.upstream.revision}:${file.upstream_path}`);
        if (sha(original) !== file.upstream_sha256) throw new Error(`Pinned source mismatch: ${file.path}`);
      }
    } else if (file.state === 'copied' || file.state === 'modified') throw new Error(`Missing source correspondence: ${file.path}`);
    if (file.state === 'copied') {
      copies++;
      if (file.sha256 !== file.upstream_sha256 || original && !maintained.equals(original)) throw new Error(`Copied source differs: ${file.path}`);
    }
    if (file.patch) {
      const patchFile = safe(file.patch), patch = readFileSync(patchFile);
      if (sha(patch) !== file.patch_sha256) throw new Error(`Patch byte mismatch: ${file.path}`);
      if (source) {
        if (!original) throw new Error(`Patch lacks source object: ${file.path}`);
        const before = join(temp, 'original'), after = join(temp, 'maintained'), applied = join(temp, 'applied');
        writeFileSync(before, original); writeFileSync(after, maintained);
        const diff = spawnSync('diff', ['-u','--label',`a/${file.path}`,'--label',`b/${file.path}`,before,after], { maxBuffer: 32 * 1024 * 1024 });
        if (diff.status !== 1 || !patch.equals(diff.stdout)) throw new Error(`Patch does not describe maintained source: ${file.path}; regenerate canonical diff`);
        const reproduction = join(temp, file.path);
        mkdirSync(dirname(reproduction), { recursive: true });
        writeFileSync(reproduction, original);
        execFileSync('git', ['apply', '--whitespace=nowarn', patchFile], { cwd: temp, maxBuffer: 1024 * 1024 });
        if (!readFileSync(reproduction).equals(maintained)) throw new Error(`Patch reproduction differs: ${file.path}`);
      }
    } else if (file.state === 'modified') throw new Error(`Modified source lacks patch: ${file.path}`);
    if (file.state === 'modified') modifications++;
  }
  const closurePath=resolve(root,'vendor/cua/dependency-closure.json');
  if(existsSync(closurePath)) {
    const closure=JSON.parse(readFileSync(closurePath));
    for(const [unit,record]of Object.entries(closure.units??{})) {
      if(sha(readFileSync(safe(record.manifest)))!==record.manifest_sha256)throw new Error(`Dependency audit manifest stale: ${unit}`);
      const lock=resolve(root,record.manifest,'..','Cargo.lock');
      if(record.lockfile_sha256&&sha(readFileSync(lock))!==record.lockfile_sha256)throw new Error(`Dependency audit lock stale: ${unit}`);
    }
  }
  const inventoryPath=resolve(root,'vendor/cua/inventory.json');
  if(existsSync(inventoryPath)) {
    const inventory=JSON.parse(readFileSync(inventoryPath));
    if(inventory.upstream_revision!==ledger.upstream.revision)throw new Error('Inventory revision differs');
    if(source)for(const pkg of inventory.packages){
      if(sha(git('show',`${ledger.upstream.revision}:${pkg.manifest}`))!==pkg.manifest_sha256)throw new Error(`Inventory manifest mismatch: ${pkg.manifest}`);
      if(pkg.license_file&&sha(git('show',`${ledger.upstream.revision}:${pkg.license_file}`))!==pkg.license_sha256)throw new Error(`Inventory notice mismatch: ${pkg.license_file}`);
    }
  }
  for (const entry of ledger.source_evidence ?? []) {
    if (source && sha(git('show', `${ledger.upstream.revision}:${entry.path}`)) !== entry.sha256) throw new Error(`Translation source mismatch: ${entry.path}`);
  }
  if (paths.has('vendor/cua/copy-plan.json')) {
    if (!paths.has('vendor/cua/copy-inventory.json')) throw new Error('Whole import inventory is not pinned in the ledger');
    const inventory = JSON.parse(readFileSync(safe('vendor/cua/copy-inventory.json')));
    if (inventory.revision !== ledger.upstream.revision) throw new Error('Whole import revision differs');
    const imported = new Set();
    for (const file of inventory.files) {
      if (!file.path.startsWith('vendor/cua/upstream/') || imported.has(file.path)) throw new Error('Unsafe or duplicate whole import path');
      imported.add(file.path);
      let bytes;
      if (file.mode === '120000') {
        const target = resolve(root,file.path);
        if (!lstatSync(target).isSymbolicLink() || !realpathSync(target).startsWith(`${root}/vendor/cua/upstream/`)) throw new Error('Whole import link escapes');
        bytes = Buffer.from(readlinkSync(target));
      } else bytes = readFileSync(safe(file.path));
      if (sha(bytes) !== file.sha256) throw new Error(`Whole import byte mismatch: ${file.path}`);
    }
    wholeImportFiles = imported.size;
    if (source) execFileSync(process.execPath, [safe('vendor/cua/copy-upstream.mjs'), source, '--root', root, '--verify'],
      {cwd:root, maxBuffer:1024*1024});
  }
} finally { rmSync(temp, { recursive: true, force: true }); }
console.log(JSON.stringify({ verified_files: paths.size, copied: copies, modified: modifications,
  whole_import_files: wholeImportFiles,
  pinned_source_checked: Boolean(source), patches_reproduced: source ? modifications : 0,
  translation_source_objects: ledger.source_evidence?.length ?? 0 }));
