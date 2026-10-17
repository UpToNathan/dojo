#!/usr/bin/env node
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { readFileSync, realpathSync } from 'node:fs';
import { resolve } from 'node:path';

const args = process.argv.slice(2);
let expected;
if (args[0] === '--pins') {
  if (!args[1]) throw new Error('--pins requires a provenance file');
  expected = JSON.parse(readFileSync(args[1], 'utf8')).observed_runtime;
  if (!expected?.executable || !expected?.executable_sha256) throw new Error('Invalid observed runtime pin');
  args.splice(0, 2);
}
const binaries = args;
if (!binaries.length) {
  process.stderr.write('Usage: probe-electron-runtime.mjs [--pins PROVENANCE.json] /absolute/path/to/electron-binary [...]\n');
  process.exit(2);
}
const runtimes = binaries.map(input => {
  if (!input.startsWith('/')) throw new Error('Supply an absolute executable path');
  const executable = realpathSync(resolve(input));
  const before = readFileSync(executable);
  if (!before.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    throw new Error(`Expected a native ELF executable: ${executable}`);
  }
  // Wire format: electron/fuses 67d4eed3a8da0563c9afeac3229c09498d326435,
  // dist/constants.js and dist/index.js. RunAsNode is V1 option zero.
  // Refuse before execution: disabled Node mode would open the application.
  const sentinel = Buffer.from('dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX');
  const offset = before.indexOf(sentinel);
  if (offset < 0 || before.indexOf(sentinel, offset + 1) >= 0) {
    throw new Error(`Missing or ambiguous Electron fuse wire: ${executable}`);
  }
  const wire = offset + sentinel.length;
  const length = before[wire + 1];
  if (before[wire] !== 1 || !length || wire + 2 + length > before.length) {
    throw new Error(`Unsupported or truncated Electron fuse wire: ${executable}`);
  }
  if (before[wire + 2] !== 49) {
    throw new Error(`RunAsNode fuse is not enabled; refusing to execute: ${executable}`);
  }
  // Electron's documented Node mode reads versions without opening a window or profile.
  const versions = JSON.parse(execFileSync(executable, ['-p', 'JSON.stringify(process.versions)'], {
    env: { ...process.env, NODE_OPTIONS: '', ELECTRON_RUN_AS_NODE: '1' },
    encoding: 'utf8', timeout: 15000, maxBuffer: 65536,
  }));
  if (!versions.electron || !versions.chrome) throw new Error('Executable did not identify an Electron/Chromium runtime');
  const after = readFileSync(executable);
  if (!before.equals(after)) throw new Error('Executable changed during runtime probe');
  return {
    executable,
    executable_sha256: createHash('sha256').update(before).digest('hex'),
    electron: versions.electron,
    chromium: versions.chrome,
    node: versions.node,
    exact_binding_verified: false,
    note: 'Runtime identity only. This probe does not establish accessibility or window binding.',
  };
});
if (expected) {
  const actual = runtimes.find(runtime => runtime.executable === expected.executable);
  if (!actual || ['executable_sha256', 'electron', 'chromium', 'node'].some(key => actual[key] !== expected[key])) {
    throw new Error('Observed Electron runtime differs from its investigated pin; re-ground before applying a producer');
  }
}
process.stdout.write(`${JSON.stringify({ schema_version: 1, runtimes }, null, 2)}\n`);
