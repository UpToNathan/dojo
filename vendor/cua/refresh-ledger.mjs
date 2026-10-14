#!/usr/bin/env node
// Refresh owned hashes and declared modifications; copied source is immutable.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, readdirSync, existsSync, mkdirSync, lstatSync, mkdtempSync, rmSync, realpathSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync, spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';

const defaultRoot = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let root = defaultRoot, source, write = false;
for (let i = 2; i < process.argv.length; i++) {
  const arg = process.argv[i];
  if (arg === '--source') source = resolve(process.argv[++i]);
  else if (arg === '--root') root = resolve(process.argv[++i]);
  else if (arg === '--write') write = true;
  else throw new Error('Usage: refresh-ledger.mjs --source PINNED_CHECKOUT [--write] [--root TEST_ROOT]');
}
if (!source) throw new Error('--source is required; never infer upstream bytes from maintained files');
root = realpathSync(root);
const ledgerPath = resolve(root, 'vendor/cua/ledger.json');
const ledger = JSON.parse(readFileSync(ledgerPath));
const revision = ledger.upstream.revision;
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
function git(...args) { return execFileSync('git', ['-C', source, ...args], { maxBuffer: 32 * 1024 * 1024 }); }
if (git('rev-parse', 'HEAD').toString().trim() !== revision) throw new Error('Source checkout HEAD differs from pinned revision');
function safe(path) {
  const file = resolve(root, path), rel = relative(root, file);
  if (!rel || rel.startsWith('..') || isAbsolute(rel)) throw new Error(`Unsafe path: ${path}`);
  if (existsSync(file) && (lstatSync(file).isSymbolicLink() || relative(root, realpathSync(file)).startsWith('..'))) throw new Error(`Symlink path refused: ${path}`);
  return file;
}
function bytes(path) { const file = safe(path); if (!lstatSync(file).isFile()) throw new Error(`Not a regular file: ${path}`); return readFileSync(file); }
const changes = [], patches = new Map();
const files = new Map(ledger.files.map(file => [file.path, { ...file }]));
if(files.size!==ledger.files.length)throw new Error('Duplicate ledger paths are refused before refresh');
// Validate every existing pinned record before collecting any new files or writing output.
for (const file of files.values()) {
  const maintained = bytes(file.path);
  if (file.state === 'copied') {
    if (!file.upstream_path || !file.upstream_sha256) throw new Error(`Copied record lacks provenance: ${file.path}`);
    const upstream = git('show', `${revision}:${file.upstream_path}`);
    if (sha(upstream) !== file.upstream_sha256 || sha(maintained) !== file.sha256 || sha(maintained) !== sha(upstream)) {
      throw new Error(`Changed copied source refused: ${file.path}; explicitly review and declare a modification first`);
    }
  } else if (file.state === 'modified') {
    if (!file.upstream_path || !file.upstream_sha256) throw new Error(`Modified record lacks provenance: ${file.path}`);
    if (sha(git('show', `${revision}:${file.upstream_path}`)) !== file.upstream_sha256) throw new Error(`Pinned source hash mismatch: ${file.path}`);
  } else if (file.state !== 'local' && file.state !== 'translated') throw new Error(`Unknown ledger state: ${file.state}`);
}
const ownedRoots = ['crates/dojo-cua-primitives', 'crates/dojo-computer', 'crates/dojo-computer-models',
  'crates/dojo-computer-verification', 'crates/dojo-capture-wayland', 'crates/dojo-accessibility',
  'crates/dojo-agent-setup', 'crates/dojo-icon-cache', 'crates/dojo-computer-recording', 'crates/dojo-external-admission', 'crates/dojo-cua-worker', 'crates/dojo-native-seat'];
function walk(path, unit = path.split('/')[1]) {
  for (const entry of readdirSync(safe(path), { withFileTypes: true })) {
    if (['target', '.git', 'node_modules', '.devenv', '.direnv'].includes(entry.name)) continue;
    const local = `${path}/${entry.name}`;
    if (entry.isSymbolicLink()) throw new Error(`Symlink in owned source: ${local}`);
    if (entry.isDirectory()) walk(local, unit);
    else if (entry.isFile() && !files.has(local)) files.set(local, { path: local, state: 'local', unit, owner: ledger.owner });
  }
}
for (const path of ownedRoots) if (existsSync(safe(path))) walk(path);
const glue = ['Cargo.toml', 'Cargo.lock', 'src/lib.rs', 'src/application_catalogue.rs', 'src/launcher.rs', 'src/agent_input.rs', 'src/bin/dojo-agent.rs',
  'vendor/cua/copy-upstream.mjs', 'vendor/cua/copy-upstream.test.mjs', 'vendor/cua/copy-plan.json', 'vendor/cua/copy-inventory.json',
  'vendor/cua/metadata/sdk-Cargo.lock', 'vendor/cua/metadata/pi-resource-catalog.json',
  'vendor/cua/metadata/niri-driver.patch', 'vendor/cua/metadata/driver-upstream-Cargo.lock',
  'vendor/cua/metadata/immutable-acquisition.patch',
  'vendor/cua/copy-first.md',
  'tools/speech/cua-extension.mjs', 'tools/speech/cua-extension.test.mjs', 'tools/speech/check-cua-catalog.mjs',
  'src/bin/dojo-computer-verification.rs', 'src/bin/dojo-computer-recording.rs', 'src/bin/dojo-computer-models.rs',
  'src/bin/dojo-agent/service.rs', 'src/bin/dojo-agent/screenshot.rs', 'src/bin/dojo-agent/tools.rs',
  'tools/speech/pi-runtime.mjs', 'tools/speech/desktop-extension.mjs', 'tools/speech/computer-models.mjs',
  'tools/speech/pi-runtime.test.mjs', 'tools/speech/pi-desktop.test.mjs', 'tools/speech/desktop-extension.test.mjs',
  'tools/speech/recording.mjs', 'tools/speech/pi-recording.mjs',
  'tools/speech/image-retention.mjs', 'tools/speech/image-retention.test.mjs', 'tools/speech/human-requests.mjs', 'tools/speech/human-requests.test.mjs',
  'tools/speech/desktop-trace.mjs', 'tools/speech/desktop-trace.test.mjs',
  'tools/speech/computer-model-bridge.mjs', 'tools/speech/computer-model-bridge.test.mjs',
  'tools/speech/computer-steps.mjs', 'tools/speech/computer-steps.test.mjs',
  'vendor/cua/provenance/dojo-owned-browser.md', 'vendor/cua/provenance/dojo-driver-service.md', 'scripts/build-cua-driver.sh',
  'vendor/niri/Cargo.toml', 'vendor/niri/Cargo.lock', 'vendor/niri/src/scene_capture.rs', 'vendor/niri/src/agent_input.rs', 'vendor/niri/src/agent_input/coordination.rs',
  'vendor/niri/src/backend/winit.rs', 'vendor/niri/src/layout/scrolling.rs', 'vendor/niri/src/layout/tests.rs', 'vendor/niri/src/agent_input/motion.rs', 'vendor/niri/src/agent_input/shared.rs', 'vendor/niri/src/input/mod.rs',
  'vendor/niri/src/agent_input/peer.rs', 'vendor/niri/src/browser_control.rs', 'vendor/niri/src/external_admission.rs', 'vendor/niri/src/external_admission/registration.rs', 'vendor/niri/src/niri.rs',
  'vendor/niri/src/ipc/server.rs', 'vendor/niri/niri-ipc/src/lib.rs', 'vendor/niri/niri-ipc/src/socket.rs', 'vendor/niri/niri-ipc/src/state.rs', 'tests/capture_desktop.rs',
  'tests/side_channel_desktop.rs', 'tests/side_channel_admission.rs', 'tests/browser_owned_desktop.rs', 'tests/desktop_policy.rs', 'tests/support/nested_workspace.rs',
  'provenance/artifacts/rapidfuzz-reference.lock.json', 'licenses/rapidfuzz-MIT.txt',
  'scripts/check-cua-metrics-reference.py', 'scripts/check-perception-artifacts.mjs',
  'scripts/check-perception-worker.mjs', 'scripts/stage-perception-installation.mjs'];
for (const path of glue) if (existsSync(safe(path)) && !files.has(path)) files.set(path, { path, state: 'local', unit: 'dojo-root-integration', owner: ledger.owner });
if (existsSync(safe('src/bin/dojo-agent'))) walk('src/bin/dojo-agent', 'dojo-root-integration');
// Only declared Dojo additions enter the private imported core as local files.
// An existing upstream object can never be laundered into that category.
const driverAdditions = new Set();
if (existsSync(safe('vendor/cua/driver/patches.json'))) {
  for (const path of JSON.parse(bytes('vendor/cua/driver/patches.json')).new_source ?? []) driverAdditions.add(path);
}
for (const path of driverAdditions) {
  const local = `vendor/cua/driver/${path}`;
  if (!existsSync(safe(local)) || files.has(local)) continue;
  if (!/^crates\/cua-driver-core\/src\/[a-zA-Z0-9_/-]+\.rs$/.test(path) || path.includes('..')) throw new Error(`Invalid declared driver addition: ${path}`);
  const original = `libs/cua-driver/rust/${path}`;
  const check = spawnSync('git', ['-C', source, 'cat-file', '-e', `${revision}:${original}`]);
  if (check.status === 0) throw new Error(`Driver addition exists upstream: ${local}; declare a copied or modified record instead`);
  files.set(local, {path: local, state: 'local', unit: 'cua-driver-private', owner: ledger.owner});
}
const tools = ['vendor/cua/refresh-ledger.mjs', 'vendor/cua/verify-ledger.mjs', 'vendor/cua/audit-dependencies.mjs', 'vendor/cua/ledger-tools.test.mjs'];
for (const path of tools) if (existsSync(safe(path)) && !files.has(path)) files.set(path, { path, state: 'local', unit: 'ledger-tooling', owner: ledger.owner });
// Declare concrete source correspondences for adapted setup/icon units and archived originals.
const correspondences = [];
for (const name of ['edit', 'fsutil', 'registry', 'state', 'skills', 'lib']) {
  correspondences.push([`crates/dojo-agent-setup/src/${name}.rs`, `libs/cua/crates/cua-agent-setup/src/${name}.rs`, 'dojo-agent-setup']);
}
correspondences.push(['vendor/cua/driver/upstream/platform-linux/browser_platform.rs', 'libs/cua-driver/rust/crates/platform-linux/src/browser_platform.rs', 'cua-driver-private']);
correspondences.push(['crates/dojo-agent-setup/tests/agents.rs', 'libs/cua/crates/cua-agent-setup/tests/agents.rs', 'dojo-agent-setup']);
for (const [local, original] of [['src/cache.rs','src/lib.rs'], ['src/cache_tests.rs','src/tests.rs'],
  ['provenance/lib.rs','src/lib.rs'], ['provenance/tests.rs','src/tests.rs'], ['provenance/Cargo.toml','Cargo.toml']]) {
  correspondences.push([`crates/dojo-icon-cache/${local}`, `libs/cua/crates/cua-icon-cache/${original}`, 'dojo-icon-cache']);
}
for (const [local, original] of [['expectation.rs','src/expectation.rs'],['verification.rs','src/contract/verification.rs']]) {
  const upstream = original === 'src/contract/verification.rs' ? 'libs/cua-driver/rust/crates/cua-driver-contract/src/verification.rs' : `libs/cua-driver/rust/crates/cua-driver-core/${original}`;
  correspondences.push([`crates/dojo-computer-verification/provenance/${local}`, upstream, 'dojo-computer-verification']);
}
correspondences.push(['crates/dojo-cua-worker/src/worker.rs', 'libs/cua-driver/rust/crates/cua-driver-sdk/src/worker.rs', 'dojo-cua-worker']);
correspondences.push(['crates/dojo-cua-worker/upstream/worker.rs', 'libs/cua-driver/rust/crates/cua-driver-sdk/src/worker.rs', 'dojo-cua-worker']);
for (const [path, upstream_path, unit] of correspondences) {
  if (!existsSync(safe(path))) continue;
  const previous = files.get(path);
  if (previous?.upstream_path && previous.upstream_path !== upstream_path) throw new Error(`Conflicting source correspondence: ${path}`);
  const upstream = git('show', `${revision}:${upstream_path}`), maintained = bytes(path);
  const state = sha(upstream) === sha(maintained) ? 'copied' : 'modified';
  if (previous?.state === 'copied' && state !== 'copied') throw new Error(`Changed copied source refused: ${path}`);
  files.set(path, { ...previous, path, unit, owner: ledger.owner, state, upstream_path, upstream_sha256: sha(upstream), sha256: sha(maintained) });
}
const temp = mkdtempSync(join(tmpdir(), 'dojo-ledger-patches-'));
try {
  for (const file of files.values()) {
    const maintained = bytes(file.path), digest = sha(maintained);
    if(file.state==='copied'&&digest!==file.upstream_sha256)throw new Error(`Copied source changed during refresh: ${file.path}`);
    if (file.sha256 !== digest) changes.push(file.path);
    if (file.state === 'modified') {
      const upstream = git('show', `${revision}:${file.upstream_path}`);
      if (sha(upstream) !== file.upstream_sha256) throw new Error(`Pinned source mismatch: ${file.path}`);
      const oldFile = join(temp, 'original'), newFile = join(temp, 'maintained');
      writeFileSync(oldFile, upstream); writeFileSync(newFile, maintained);
      const diff = spawnSync('diff', ['-u', '--label', `a/${file.path}`, '--label', `b/${file.path}`, oldFile, newFile], { maxBuffer: 32 * 1024 * 1024 });
      if (diff.status !== 0 && diff.status !== 1) throw new Error(`diff failed: ${file.path}`);
      if (!diff.stdout.length) throw new Error(`Modified file matches source; explicitly reclassify it: ${file.path}`);
      file.patch = `vendor/cua/ledger-patches/${file.path}.patch`;
      file.patch_sha256 = sha(diff.stdout);
      patches.set(file.patch, diff.stdout);
    }
    file.sha256 = digest;
  }
} finally { rmSync(temp, { recursive: true, force: true }); }
ledger.files = [...files.values()].sort((a,b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
ledger.schema_version = 2;
ledger.refresh = { command: 'node vendor/cua/refresh-ledger.mjs --source PINNED_CHECKOUT --write',
  policy: 'Only local/translated and explicitly modified records are refreshed. Existing copied records must still equal both recorded and pinned bytes. Unified patches compare pinned git objects to maintained source.' };
ledger.source_evidence = ledger.source_evidence ?? [];
// Behavioral translations may span many source files. Record those source objects without pretending there is a literal one-file copy.
for (const manifest of ['crates/dojo-computer-models/provenance.json','crates/dojo-computer-verification/provenance.json','crates/dojo-computer-recording/provenance.json','crates/dojo-accessibility/provenance.json','crates/dojo-capture-wayland/provenance.json']) {
  if (!existsSync(safe(manifest))) continue;
  const record = JSON.parse(bytes(manifest));
  for (const item of record.sources ?? record.upstream_sources ?? record.upstream_files ?? []) {
    const path = typeof item === 'string' ? item : item.path;
    if (!path) continue;
    const digest = sha(git('show', `${revision}:${path}`));
    if (item.sha256 && item.sha256 !== digest) throw new Error(`Translation source hash mismatch: ${path}`);
    if (!ledger.source_evidence.some(entry => entry.path === path)) ledger.source_evidence.push({ path, sha256: digest, declared_by: manifest });
  }
}
for (const [path, declared_by, method] of [
  ['libs/cua-driver/rust/crates/platform-linux/src/tools/impl_.rs','src/bin/dojo-agent/discovery.rs', 'Bounded Rust translation of Linux discovery and merge semantics'],
  ['libs/cua-driver/rust/crates/platform-linux/src/proc_fs.rs','src/bin/dojo-agent/discovery.rs', 'Bounded Rust translation of Linux discovery and merge semantics'],
  ['libs/cua-driver/rust/crates/platform-linux/src/installed_apps.rs','src/application_catalogue.rs', 'Shared catalogue metadata adapted from the pinned Linux implementation'],
  ['libs/python/agent/cua_agent/callbacks/image_retention.py','tools/speech/image-retention.mjs'],
  ['libs/python/agent/cua_agent/human_tool/server.py','tools/speech/human-requests.mjs'],
  ['libs/python/agent/cua_agent/adapters/human_adapter.py','tools/speech/human-requests.mjs'],
]) {
  if(existsSync(safe(declared_by))&&!ledger.source_evidence.some(entry=>entry.path===path))
    ledger.source_evidence.push({path,sha256:sha(git('show',`${revision}:${path}`)),declared_by,
      method: method ?? 'Behavior reference for thin Pi-owned adapter; no upstream scheduler/server imported'});
}
ledger.source_evidence.sort((a,b) => a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
if (write) {
  for (const [path, contents] of patches) { mkdirSync(dirname(safe(path)), { recursive: true }); writeFileSync(safe(path), contents); }
  writeFileSync(ledgerPath, JSON.stringify(ledger, null, 2) + '\n');
}
console.log(JSON.stringify({ written: write, files: ledger.files.length, copied: ledger.files.filter(f => f.state === 'copied').length,
  modified: ledger.files.filter(f => f.state === 'modified').length, source_evidence: ledger.source_evidence.length, regenerated_patches: patches.size, changed_owned_hashes: changes.length }));
