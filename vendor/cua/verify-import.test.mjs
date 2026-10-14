// Tests the local verification boundary, not CUA behavior or desktop effects.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';

const script = readFileSync(new URL('./verify-import.mjs', import.meta.url));
const sha = b => createHash('sha256').update(b).digest('hex');
function fixture(t, mutate = () => {}) {
  const root = mkdtempSync(join(tmpdir(), 'cua-ledger-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'vendor/cua'), { recursive: true });
  writeFileSync(join(root, 'vendor/cua/verify-import.mjs'), script);
  writeFileSync(join(root, 'input.rs'), 'upstream bytes\n');
  const ledger = { upstream: { revision: 'invalid' }, files: [{ path: 'input.rs', state: 'local', sha256: sha('upstream bytes\n') }] };
  mutate(ledger, root);
  writeFileSync(join(root, 'vendor/cua/ledger.json'), JSON.stringify(ledger));
  return { root, run: (...args) => spawnSync(process.execPath, [join(root, 'vendor/cua/verify-import.mjs'), ...args], { encoding: 'utf8' }) };
}
test('byte verification requires no source checkout or network', t => {
  const result = fixture(t).run();
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Verified 1 maintained files offline/);
});
test('changed maintained bytes fail closed', t => {
  const f = fixture(t);
  writeFileSync(join(f.root, 'input.rs'), 'tampered');
  assert.match(f.run().stderr, /Local byte mismatch/);
});
test('patch bytes are also pinned', t => {
  const f = fixture(t, (l, root) => {
    writeFileSync(join(root, 'patch'), 'changed patch');
    l.files[0].patch = 'patch';
    l.files[0].patch_sha256 = sha('original patch');
  });
  assert.match(f.run().stderr, /Patch mismatch/);
});
test('ledger cannot read outside the maintained root', t => {
  const f = fixture(t, l => { l.files[0].path = '../outside'; });
  assert.match(f.run().stderr, /Unsafe path/);
});
test('extraction requires the pinned source', t => {
  const f = fixture(t);
  assert.match(f.run('--extract', join(f.root, 'output')).stderr, /requires --source/);
});
test('unknown flags are not silently ignored', t => {
  assert.match(fixture(t).run('--overwrite').stderr, /Usage:/);
});
