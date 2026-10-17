#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const directory = mkdtempSync(join(tmpdir(), 'dojo-probe-safety-'));
try {
  const hook = join(directory, 'deny-execution.mjs');
  writeFileSync(hook, `import cp from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
cp.execFileSync = () => { throw new Error('UNSAFE_EXECUTION_ATTEMPT'); };
syncBuiltinESMExports();\n`);
  const sentinel = Buffer.from('dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX');
  const elf = Buffer.from([0x7f, 0x45, 0x4c, 0x46]);
  const cases = [
    ['missing', elf, /Missing or ambiguous/],
    ['disabled', Buffer.concat([elf, sentinel, Buffer.from([1, 1, 48])]), /RunAsNode fuse is not enabled/],
    ['removed', Buffer.concat([elf, sentinel, Buffer.from([1, 1, 114])]), /RunAsNode fuse is not enabled/],
    ['truncated', Buffer.concat([elf, sentinel, Buffer.from([1, 2, 49])]), /Unsupported or truncated/],
    ['unknown', Buffer.concat([elf, sentinel, Buffer.from([2, 1, 49])]), /Unsupported or truncated/],
    ['ambiguous', Buffer.concat([elf, sentinel, Buffer.from([1, 1, 49]), sentinel]), /Missing or ambiguous/],
    ['enabled', Buffer.concat([elf, sentinel, Buffer.from([1, 1, 49])]), /UNSAFE_EXECUTION_ATTEMPT/],
  ];
  for (const [name, bytes, expected] of cases) {
    const executable = join(directory, name);
    writeFileSync(executable, bytes);
    const result = spawnSync(process.execPath, ['--import', hook, 'tools/accessibility/probe-electron-runtime.mjs', executable], { encoding: 'utf8' });
    assert.equal(result.status, 1, name);
    assert.match(result.stderr, expected, name);
    if (name !== 'enabled') assert.doesNotMatch(result.stderr, /UNSAFE_EXECUTION_ATTEMPT/, name);
  }
  console.log('Passed seven runtime probe execution-boundary cases.');
} finally {
  rmSync(directory, { recursive: true, force: true });
}
