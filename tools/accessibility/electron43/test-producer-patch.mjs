#!/usr/bin/env node
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
import { dirname, resolve, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { execFileSync, spawnSync } from 'node:child_process';

const directory = dirname(fileURLToPath(import.meta.url));
const manifest = JSON.parse(readFileSync(join(directory, 'chromium-producer-provenance.json')));
const checkout = process.argv[2];
if (!checkout || process.argv.length !== 3) {
  console.error('Usage: node test-producer-patch.mjs <exact Chromium150 source preimages>');
  process.exit(2);
}
const fixture = mkdtempSync(join(tmpdir(), 'dojo-electron43-patch-'));
try {
  for (const { path } of manifest.upstream_inputs) {
    const destination = join(fixture, path);
    mkdirSync(dirname(destination), { recursive: true });
    // The local subset may have the producer applied. Read committed upstream bytes.
    writeFileSync(destination, execFileSync('git', ['show', `HEAD:${path}`], { cwd: checkout }));
  }
  const check = () => spawnSync(process.execPath,
    [join(directory, 'check-producer-patch.mjs'), fixture], { encoding: 'utf8' });
  let result = check();
  assert.equal(result.status, 0, result.stderr);
  const connection = join(fixture, 'ui/ozone/platform/wayland/host/wayland_connection.cc');
  const original = readFileSync(connection, 'utf8');
  const from = 'if (auto* pointer = seat_->pointer()) {';
  assert.equal(original.split(from).length, 2);
  writeFileSync(connection, original.replace(from,
    'if (auto* pointer = seat_ ? seat_->pointer() : nullptr) {'));
  result = check();
  assert.equal(result.status, 0, result.stderr);
  writeFileSync(connection, readFileSync(connection, 'utf8') + '\n// deliberate source drift\n');
  result = check();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Pinned source differs/);
  writeFileSync(connection, original.replace(from,
    'if (auto* pointer = seat_ ? seat_->pointer() : nullptr) {'));
  execFileSync('git', ['apply', join(directory, manifest.patch.file)], { cwd: fixture });
  result = check();
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Pinned source differs/);
  console.log('Passed: upstream preimages; Electron teardown overlap; drift refusal; already-applied refusal');
} finally {
  rmSync(fixture, { recursive: true, force: true });
}
