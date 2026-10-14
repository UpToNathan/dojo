#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';

const root = path.resolve(import.meta.dirname, '..');
const [archive, artifacts] = process.argv.slice(2);
if (!archive || !artifacts || process.argv.length !== 4) throw new Error('Usage: stage-perception-installation.mjs VERIFIED_ARCHIVE EXTRACTED_ARTIFACTS');
const run = (...args) => execFileSync('devenv', ['shell', '--', ...args], {cwd: root, encoding: 'utf8', maxBuffer: 8 * 1024 * 1024});
execFileSync(process.execPath, ['scripts/check-perception-artifacts.mjs', archive, artifacts], {cwd: root, stdio: 'inherit'});
const ledger = JSON.parse(fs.readFileSync(path.join(root, 'vendor/cua/ledger.json')));
if (ledger.upstream.revision !== '88aff7e5424e9ed45b93ded85eb4111472ca5190') throw new Error('Worker baseline changed');
const workerSources = ledger.files.filter(file => file.path.startsWith('vendor/cua/driver/crates/cua-perception/'));
if (workerSources.length !== 46) throw new Error('Worker source closure differs');
for (const file of workerSources) {
  const digest = crypto.createHash('sha256').update(fs.readFileSync(path.join(root, file.path))).digest('hex');
  if (file.state !== 'copied' || digest !== file.upstream_sha256) throw new Error(`Worker source changed: ${file.path}`);
}
run('cargo', 'build', '--locked', '--manifest-path', 'vendor/cua/driver/Cargo.toml', '-p', 'cua-perception');
const executable = path.join(root, 'vendor/cua/driver/target/debug/cua-perception');
const interpreter = run('patchelf', '--print-interpreter', executable).trim().split('\n').at(-1);
const gccLibrary = run('gcc', '-print-file-name=libstdc++.so.6').trim().split('\n').at(-1);
if (!gccLibrary.startsWith('/nix/store/') || !interpreter.startsWith('/nix/store/')) throw new Error('Expected immutable Nix compiler/interpreter');
const loader = fs.realpathSync(path.dirname(gccLibrary));
const dependencies = run('env', `LD_LIBRARY_PATH=${loader}`, 'ldd', executable, path.join(artifacts, 'runtime/libonnxruntime.so'));
if (dependencies.includes('not found')) throw new Error('Runtime loader closure unresolved');
const libraries = [...new Set([...dependencies.matchAll(/(?:=>\s*)?(\/nix\/store\/[^\s()]+)/g)].map(match => fs.realpathSync(match[1])))];
if (!libraries.length || libraries.some(file => !fs.statSync(file).isFile())) throw new Error('Invalid runtime loader closure');
const store = file => {
  const value = run('nix-store', '--add', file).trim().split('\n').at(-1);
  if (!value.startsWith('/nix/store/')) throw new Error('Nix did not return a store path');
  return value;
};
const bundle = store(path.resolve(artifacts));
const stagedWorker = fs.mkdtempSync(path.join(os.tmpdir(), 'dojo-perception-worker-'));
let worker;
try {
  const directory = path.join(stagedWorker, 'worker');
  fs.mkdirSync(path.join(directory, 'bin'), {recursive: true});
  fs.copyFileSync(executable, path.join(directory, 'bin/cua-perception'));
  fs.chmodSync(path.join(directory, 'bin/cua-perception'), 0o555);
  worker = path.join(store(directory), 'bin/cua-perception');
} finally { fs.rmSync(stagedWorker, {recursive: true, force: true}); }
const config = {
  source_commit: '88aff7e5424e9ed45b93ded85eb4111472ca5190',
  executable: worker,
  executable_sha256: crypto.createHash('sha256').update(fs.readFileSync(worker)).digest('hex'),
  manifest: path.join(bundle, 'model-manifest.json'),
  runtime: path.join(bundle, 'runtime/libonnxruntime.so'),
  interpreter,
  libraries,
  loader_search_paths: [...new Set(libraries.map(file => path.dirname(file)))],
};
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), 'dojo-perception-installation-'));
try {
  const file = path.join(temporary, 'dojo-perception-installation.json');
  fs.writeFileSync(file, JSON.stringify(config, null, 2) + '\n', {mode: 0o600});
  console.log(JSON.stringify({installation: store(file), executable_sha256: config.executable_sha256, libraries: libraries.length, deployed: false}));
} finally { fs.rmSync(temporary, {recursive: true, force: true}); }
