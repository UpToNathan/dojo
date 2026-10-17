#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const directory = import.meta.dirname;
const repo = resolve(directory, '../../..');
const pins = JSON.parse(readFileSync(join(repo, 'provenance/native-electron-investigation.json')));
const producer = JSON.parse(readFileSync(join(directory, 'chromium-producer-provenance.json')));
const [mode] = process.argv.slice(2);
assert(process.argv.length === 3 && ['--hooks', '--configure', '--objects', '--compile'].includes(mode), 'Use --hooks, --configure, --objects or --compile');
const workspace = join(homedir(), '.cache/dojo/electron-build', pins.electron.version);
const source = join(workspace, 'src');
const electron = join(source, 'electron');
const tools = join(homedir(), '.cache/dojo/upstream/depot_tools', pins.depot_tools.revision);
const env = { ...process.env, PATH: `${tools}:${process.env.PATH}`, DEPOT_TOOLS_UPDATE: '0', DEPOT_TOOLS_METRICS: '0' };
const run = (command, args, cwd = source, capture = false) => {
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8', stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} ${args.join(' ')} failed`);
  return result.stdout?.trim();
};
const git = (cwd, args) => run('git', ['-C', cwd, ...args], source, true);
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
assert(existsSync(join(workspace, 'resolved-source-revisions.json')), 'Finish the existing dependency sync before running hooks');
assert.equal(sha(readFileSync(join(workspace, 'resolved-source-revisions.json'))), sha(readFileSync(join(directory, 'resolved-source-revisions.json'))), 'Resolved dependency revisions differ from the recorded lock');
assert.equal(git(electron, ['rev-parse', 'HEAD']), pins.electron.revision);
assert.equal(git(tools, ['rev-parse', 'HEAD']), pins.depot_tools.revision);
assert.equal(sha(readFileSync(join(electron, 'DEPS'))), pins.electron.deps_sha256);
const patch = readFileSync(join(directory, producer.patch.file));
assert.equal(sha(patch), producer.patch.sha256);
const receiptPath = join(workspace, 'dojo-hooks.json');
const identity = { electron: pins.electron.revision, chromium: pins.chromium.revision, producer_patch_sha256: producer.patch.sha256 };
if (mode === '--hooks') {
  assert(!existsSync(receiptPath), 'Hooks already completed; use --configure next');
  assert.equal(git(source, ['rev-parse', 'HEAD']), pins.chromium.revision, 'Start hooks from the synced source pin');
  run(process.execPath, [join(directory, 'check-producer-patch.mjs'), source]);
  const patchName = 'dojo_wayland_atspi_annotation.patch';
  const seriesPath = join(electron, 'patches/chromium/.patches');
  const original = git(electron, ['show', 'HEAD:patches/chromium/.patches']) + '\n';
  const desired = original + patchName + '\n';
  const current = readFileSync(seriesPath, 'utf8');
  assert(current === original || current === desired, 'Refusing to overwrite another patch series edit');
  const mail = 'From 0000000000000000000000000000000000000000 Mon Sep 17 00:00:00 2001\n'
    + 'From: Dojo build <build@dojo.invalid>\nDate: Wed, 14 Oct 2026 00:00:00 +0000\n'
    + 'Subject: [PATCH] Publish exact native Wayland accessibility endpoints\n\n' + patch.toString();
  const mailPath = join(electron, 'patches/chromium', patchName);
  if (existsSync(mailPath)) assert.equal(readFileSync(mailPath, 'utf8'), mail, 'Existing producer patch differs');
  else writeFileSync(mailPath, mail);
  if (current !== desired) writeFileSync(seriesPath, desired);
  run(join(tools, 'gclient'), ['runhooks'], workspace);
  assert.equal(git(source, ['log', '-1', '--format=%s']), 'Publish exact native Wayland accessibility endpoints');
  writeFileSync(receiptPath, JSON.stringify({ ...identity, source_head: git(source, ['rev-parse', 'HEAD']),
    integration_patch_sha256: sha(mail), resolved_sources_sha256: sha(readFileSync(join(workspace, 'resolved-source-revisions.json'))) }, null, 2) + '\n');
} else {
  assert(existsSync(receiptPath), 'Complete --hooks before configuring or compiling');
  const receipt = JSON.parse(readFileSync(receiptPath));
  for (const [key, value] of Object.entries(identity)) assert.equal(receipt[key], value, 'Hooks belong to another producer revision');
  assert.equal(git(source, ['rev-parse', 'HEAD']), receipt.source_head, 'Patched source revision changed');
  const output = join(source, 'out/Dojo');
  const expectedArgs = readFileSync(join(directory, 'args.gn'), 'utf8');
  if (mode === '--configure') {
    mkdirSync(output, { recursive: true });
    const argsPath = join(output, 'args.gn');
    if (existsSync(argsPath)) assert.equal(readFileSync(argsPath, 'utf8'), expectedArgs, 'Existing build arguments differ');
    else writeFileSync(argsPath, expectedArgs);
    run(join(source, 'buildtools/linux64/gn'), ['gen', 'out/Dojo']);
  } else {
    assert.equal(readFileSync(join(output, 'args.gn'), 'utf8'), expectedArgs, 'Configured build arguments changed');
    assert(existsSync(join(output, 'build.ninja')), 'Configure before compiling');
    const targets = mode === '--objects' ? JSON.parse(readFileSync(join(directory, 'producer-object-targets.json'))) : ['electron:electron_dist_zip'];
    run('nice', ['-n', '10', join(source, 'third_party/ninja/ninja'), '-C', 'out/Dojo', '-j', process.env.DOJO_ELECTRON_JOBS ?? '4', ...targets]);
    if (mode === '--objects') {
      const objects = targets.map(target => ({ target, sha256: sha(readFileSync(join(output, target))) }));
      console.log(JSON.stringify({ compiled_producer_objects: true, objects, source_head: receipt.source_head, full_runtime_built: false }));
    } else {
      assert(existsSync(join(output, 'dist.zip')), 'The runtime distribution is missing');
      console.log(JSON.stringify({ compiled: true, distribution: join(output, 'dist.zip'), sha256: sha(readFileSync(join(output, 'dist.zip'))), source_head: receipt.source_head }));
    }
  }
}
