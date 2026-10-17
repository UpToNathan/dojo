#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const directory = dirname(fileURLToPath(import.meta.url));
const manifest = JSON.parse(readFileSync(resolve(directory, 'chromium-producer-provenance.json'), 'utf8'));
const checkout = process.argv[2];
if (!checkout || process.argv.length !== 3) {
  console.error('Usage: node check-producer-patch.mjs <pinned Chromium checkout>');
  process.exit(2);
}
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
try {
  const patch = resolve(directory, manifest.patch.file);
  if (sha256(readFileSync(patch)) !== manifest.patch.sha256) throw Error('Producer patch hash differs from provenance');
  for (const input of manifest.upstream_inputs) {
    let bytes = readFileSync(resolve(checkout, input.path));
    if (input.path === 'ui/ozone/platform/wayland/host/wayland_connection.cc') {
      // Electron's pinned teardown fix is the only overlapping source patch.
      bytes = Buffer.from(bytes.toString().replace(
        'if (auto* pointer = seat_ ? seat_->pointer() : nullptr) {',
        'if (auto* pointer = seat_->pointer()) {'));
    }
    if (sha256(bytes) !== input.sha256) throw Error(`Pinned source differs: ${input.path}`);
  }
  execFileSync('git', ['apply', '--check', patch], { cwd: checkout, stdio: 'pipe' });
  console.log(`Producer patch applies to checked source at Chromium ${manifest.chromium.commit}`);
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
