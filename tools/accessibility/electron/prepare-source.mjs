#!/usr/bin/env node
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, resolve } from 'node:path';

const repo = resolve(import.meta.dirname, '../../..');
const pins = JSON.parse(readFileSync(join(repo, 'provenance/native-electron-investigation.json')));
const args = process.argv.slice(2);
assert(args.every(arg => ['--prepare', '--sync'].includes(arg)), 'Use --prepare or --sync');
assert(args.length === 1, 'Choose exactly one mode');
const cache = join(homedir(), '.cache/dojo');
const tools = join(cache, 'upstream/depot_tools', pins.depot_tools.revision);
const electron = join(cache, 'upstream/electron', pins.electron.revision);
const workspace = join(cache, 'electron-build', pins.electron.version);
const run = (command, argv, cwd, capture = false, env = process.env) => {
  const result = spawnSync(command, argv, { cwd, env, encoding: 'utf8', stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit' });
  if (result.error) throw result.error;
  assert.equal(result.status, 0, `${command} ${argv.join(' ')} failed`);
  return result.stdout?.trim();
};
const git = (directory, argv, capture = false) => run('git', ['-C', directory, ...argv], repo, capture);
for (const [directory, source] of [[tools, pins.depot_tools], [electron, pins.electron]]) {
  assert(/^[a-f0-9]{40}$/.test(source.revision), 'Require an exact source revision');
  if (!existsSync(join(directory, '.git'))) {
    mkdirSync(directory, { recursive: true });
    git(directory, ['init', '-q']);
    git(directory, ['fetch', '--depth=1', source.repository, source.revision]);
    git(directory, ['checkout', '--detach', source.revision]);
  }
  assert.equal(git(directory, ['rev-parse', 'HEAD'], true), source.revision, 'Cached source revision changed');
  git(directory, ['diff', '--exit-code', 'HEAD', '--']);
}
assert.equal(createHash('sha256').update(readFileSync(join(electron, 'DEPS'))).digest('hex'), pins.electron.deps_sha256, 'Electron DEPS changed');
const marker = join(workspace, 'dojo-source-pins.json');
const identity = JSON.stringify({ electron: pins.electron.revision, chromium: pins.chromium.revision, depot_tools: pins.depot_tools.revision }, null, 2) + '\n';
if (existsSync(workspace)) {
  assert(existsSync(marker), 'Refusing to adopt an unmarked source workspace');
  assert.equal(readFileSync(marker, 'utf8'), identity, 'Workspace belongs to different source pins');
} else {
  mkdirSync(workspace, { recursive: true });
  writeFileSync(marker, identity);
}
const workingElectron = join(workspace, 'src/electron');
if (!existsSync(workingElectron)) {
  mkdirSync(join(workspace, 'src'), { recursive: true });
  run('git', ['clone', '--no-hardlinks', '--no-checkout', electron, workingElectron], repo);
  git(workingElectron, ['checkout', '--detach', pins.electron.revision]);
}
assert.equal(git(workingElectron, ['rev-parse', 'HEAD'], true), pins.electron.revision);
const config = `solutions = [{"name": "src/electron", "url": "${pins.electron.repository}", "managed": False, "custom_deps": {}, "custom_vars": {"chromium_version": "${pins.chromium.revision}"}}]\ntarget_os = ["linux"]\n`;
const configPath = join(workspace, '.gclient');
if (existsSync(configPath)) assert.equal(readFileSync(configPath, 'utf8'), config, 'gclient configuration changed');
else writeFileSync(configPath, config);
if (args[0] === '--sync') {
  const env = { ...process.env, PATH: `${tools}:${process.env.PATH}`, DEPOT_TOOLS_UPDATE: '0', DEPOT_TOOLS_METRICS: '0' };
  run(join(tools, 'gclient'), ['sync', '--no-history', '--shallow', '--nohooks', '--jobs=4'], workspace, false, env);
  assert.equal(git(join(workspace, 'src'), ['rev-parse', 'HEAD'], true), pins.chromium.revision, 'Chromium checkout differs from pin');
  run(join(tools, 'gclient'), ['revinfo', '--actual', '--output-json', join(workspace, 'resolved-source-revisions.json')], workspace, false, env);
  const expectedLock = join(import.meta.dirname, 'resolved-source-revisions.json');
  if (existsSync(expectedLock)) assert.equal(readFileSync(join(workspace, 'resolved-source-revisions.json'), 'utf8'), readFileSync(expectedLock, 'utf8'), 'Resolved dependencies drifted from the recorded revisions');
}
console.log(JSON.stringify({ prepared: true, dependencies_synced: args[0] === '--sync', workspace, tools, source_pins: JSON.parse(identity), producer_built: false }));
