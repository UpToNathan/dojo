#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

let root = path.resolve(import.meta.dirname, '../..');
let source, verify = false;
for (let i = 2; i < process.argv.length; i++) {
  if (process.argv[i] === '--root') root = path.resolve(process.argv[++i]);
  else if (process.argv[i] === '--verify') verify = true;
  else if (!source && !process.argv[i].startsWith('--')) source = process.argv[i];
  else throw new Error('Usage: copy-upstream.mjs PINNED_CHECKOUT [--verify] [--root TEST_ROOT]');
}
const plan = JSON.parse(fs.readFileSync(path.join(root, 'vendor/cua/copy-plan.json')));
if (!source) throw new Error('A pinned source checkout is required');
const git = (...args) => execFileSync('git', ['-C', source, ...args], {maxBuffer: 256 * 1024 * 1024});
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
if (git('rev-parse', 'HEAD').toString().trim() !== plan.revision) throw new Error('Source revision differs from the copy plan');
const destination = path.resolve(root, plan.destination);
if (!destination.startsWith(`${root}/vendor/cua/`)) throw new Error('Copy destination must be inside vendor/cua');
const exists = file => {try {fs.lstatSync(file); return true;} catch (error) {if (error.code === 'ENOENT') return false; throw error;}};
function parentDirectories(file) {
  for (let directory = path.dirname(file); directory.startsWith(`${root}/`); directory = path.dirname(directory)) {
    if (exists(directory) && fs.lstatSync(directory).isSymbolicLink()) throw new Error(`Import parent is a symlink: ${directory}`);
  }
}
const selected = [...plan.trees, ...plan.files];
const entries = git('ls-tree', '-rz', plan.revision, '--', ...selected).toString().split('\0').filter(Boolean).map(entry => {
  const [header, name] = entry.split('\t');
  const [mode, kind, blob] = header.split(' ');
  if (kind !== 'blob' || !['100644', '100755', '120000'].includes(mode)) throw new Error(`Unsupported source object: ${name}`);
  if (path.isAbsolute(name) || name.split('/').includes('..')) throw new Error(`Unsafe source path: ${name}`);
  return {name, mode, blob};
});
for (const entry of entries.filter(entry => entry.mode === '120000')) {
  const link = git('show', `${plan.revision}:${entry.name}`).toString();
  const resolved = path.posix.normalize(path.posix.join(path.posix.dirname(entry.name), link));
  if (path.posix.isAbsolute(link) || resolved.startsWith('../') || !entries.some(other =>
      other.name === resolved || other.name.startsWith(`${resolved}/`))) {
    throw new Error(`Source link escapes the selected import: ${entry.name}`);
  }
}
for (const requested of selected) {
  if (!entries.some(entry => entry.name === requested || entry.name.startsWith(`${requested}/`))) {
    throw new Error(`Pinned source has no requested path: ${requested}`);
  }
}
const stage = fs.mkdtempSync(path.join(os.tmpdir(), 'dojo-cua-copy-'));
try {
  execFileSync('tar', ['-x', '--no-same-owner', '-C', stage], {input: git('archive', '--format=tar', plan.revision, ...selected)});
  const contents = (base, entry) => entry.mode === '120000'
    ? Buffer.from(fs.readlinkSync(path.join(base, entry.name))) : fs.readFileSync(path.join(base, entry.name));
  const original = new Map(entries.map(entry => [entry.name, contents(stage, entry)]));
  for (const workspace of plan.workspaces) {
    const file = path.join(stage, workspace.file);
    const text = fs.readFileSync(file, 'utf8');
    const expression = /(^\[workspace\]\s*\n(?:[^\[]|\[(?!workspace\]))*?\bmembers\s*=\s*)\[[\s\S]*?\]/m;
    const matches = text.match(expression);
    if (!matches) throw new Error(`Workspace membership not found: ${workspace.file}`);
    fs.writeFileSync(file, text.replace(expression, (_, prefix) => `${prefix}[\n${workspace.members.map(member => `    "${member}",`).join('\n')}\n]`));
  }
  for (const adaptation of plan.adaptations ?? []) {
    if (adaptation.file !== 'libs/cua/Cargo.lock') throw new Error('Only the filtered SDK lock is an allowed metadata adaptation');
    const metadata = path.resolve(root, adaptation.metadata);
    if (!metadata.startsWith(`${root}/vendor/cua/metadata/`)) throw new Error('Metadata adaptation escapes its owned directory');
    const bytes = fs.readFileSync(metadata);
    if (sha(bytes) !== adaptation.sha256) throw new Error('Metadata adaptation hash differs');
    const packages = text => new Map(text.split('[[package]]').slice(1).map(block => {
      const field = name => block.match(new RegExp(`^${name} = "([^"\\n]+)"$`, 'm'))?.[1];
      return [[field('name'),field('version'),field('source')??'path'].join('@'),field('checksum')];
    }));
    const upstream = packages(original.get(adaptation.file).toString());
    for (const [identity, checksum] of packages(bytes.toString())) {
      if (!upstream.has(identity) || upstream.get(identity) !== checksum) throw new Error(`Adapted lock changes an upstream package identity: ${identity}`);
    }
    fs.writeFileSync(path.join(stage, adaptation.file), bytes);
  }
  for (const patch of plan.patches ?? []) {
    const metadata = path.resolve(root, patch.metadata);
    if (!metadata.startsWith(`${root}/vendor/cua/metadata/`)) throw new Error('Patch escapes its owned directory');
    parentDirectories(metadata);
    if (!fs.lstatSync(metadata).isFile()) throw new Error('Patch metadata must be a regular file');
    const bytes = fs.readFileSync(metadata);
    if (sha(bytes) !== patch.sha256) throw new Error('Patch hash differs');
    if (!patch.files?.length) throw new Error('Patch must declare every changed file');
    for (const file of patch.files) {
      if (path.isAbsolute(file.path) || file.path.split('/').includes('..') || /[\[\]*?]/.test(file.path)) throw new Error('Unsafe patch path');
      for (let directory = path.dirname(path.join(stage, file.path)); directory.startsWith(`${stage}/`); directory = path.dirname(directory)) {
        if (exists(directory) && fs.lstatSync(directory).isSymbolicLink()) throw new Error('Patch parent cannot be a symlink');
      }
      const existing = entries.find(entry => entry.name === file.path);
      if (existing?.mode === '120000') throw new Error('Patches cannot change source symlinks');
      if (!existing && !file.added) throw new Error('Patch addition must be declared');
      if (existing && file.added) throw new Error('Upstream files cannot be declared as additions');
    }
    execFileSync('git', ['apply', '--check', ...patch.files.map(file => `--include=${file.path}`), metadata], {cwd:stage});
    execFileSync('git', ['apply', ...patch.files.map(file => `--include=${file.path}`), metadata], {cwd:stage});
    for (const file of patch.files) {
      const target = path.join(stage, file.path);
      if (!fs.lstatSync(target).isFile() || sha(fs.readFileSync(target)) !== file.sha256) throw new Error(`Patched file differs: ${file.path}`);
      if (file.added) entries.push({name:file.path, mode:'100644', blob:null});
    }
  }
  // Validate the complete output before replacing anything. Reruns refuse any
  // maintained edit, including an edit to an originally copied source file.
  for (const entry of entries) {
    const target = path.join(destination, entry.name);
    parentDirectories(target);
    const bytes = contents(stage, entry);
    if (verify && !exists(target)) throw new Error(`Imported file is missing: ${entry.name}`);
    if (exists(target) && entry.mode !== '120000'
        && Boolean(fs.lstatSync(target).mode & 0o111) !== (entry.mode === '100755')) {
      throw new Error(`Imported executable mode differs: ${entry.name}`);
    }
    if (exists(target) && ((entry.mode === '120000' ? !fs.lstatSync(target).isSymbolicLink()
        : !fs.lstatSync(target).isFile()) || !contents(destination, entry).equals(bytes))) {
      throw new Error(`Existing import differs; review instead of overwriting: ${entry.name}`);
    }
  }
  const records = [];
  for (const entry of entries) {
    const bytes = contents(stage, entry);
    const target = path.join(destination, entry.name);
    if (!verify) fs.mkdirSync(path.dirname(target), {recursive: true});
    if (!verify && !exists(target)) {
      if (entry.mode === '120000') fs.symlinkSync(bytes.toString(), target);
      else fs.writeFileSync(target, bytes, {mode: entry.mode === '100755' ? 0o755 : 0o644});
    }
    records.push({path: path.relative(root, target), upstream_path: entry.name, git_blob: entry.blob,
      sha256: sha(bytes), upstream_sha256: original.has(entry.name) ? sha(original.get(entry.name)) : null, mode: entry.mode,
      state: !original.has(entry.name) ? 'local-adaptation' : bytes.equals(original.get(entry.name)) ? 'copied' : 'modified'});
  }
  const inventory = JSON.stringify({
    schema_version: 1, revision: plan.revision, scope: 'Whole Driver and MIT sandbox/SDK source imports. Integration and runtime parity are separate gates.',
    support_data: plan.support_data ?? [], patches: plan.patches ?? [], files: records
  }, null, 2) + '\n';
  const inventoryPath = path.join(root, 'vendor/cua/copy-inventory.json');
  if (verify) {
    if (fs.readFileSync(inventoryPath, 'utf8') !== inventory) throw new Error('Copy inventory differs from pinned source');
  } else fs.writeFileSync(inventoryPath, inventory);
  console.log(JSON.stringify({files: records.length, copied: records.filter(record => record.state === 'copied').length,
    modified: records.filter(record => record.state === 'modified').length,
    local_adaptations: records.filter(record => record.state === 'local-adaptation').length}));
} finally { fs.rmSync(stage, {recursive: true, force: true}); }
