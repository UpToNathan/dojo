#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const root = path.resolve(import.meta.dirname, '..');
const release = JSON.parse(fs.readFileSync(path.join(root, 'provenance/artifacts/cua-perception-release.lock.json')));
const artifacts = JSON.parse(fs.readFileSync(path.join(root, 'provenance/artifacts/cua-perception-artifacts.lock.json')));
const [archive, directory] = process.argv.slice(2);
if (!archive || !directory || process.argv.length !== 4) throw new Error('Usage: node scripts/check-perception-artifacts.mjs ARCHIVE EXTRACTED_DIRECTORY');

async function check(file, expected) {
  const stat = fs.lstatSync(file);
  if (!stat.isFile() || stat.isSymbolicLink()) throw new Error(`Not a regular artifact: ${file}`);
  if (expected.size !== undefined && stat.size !== expected.size) throw new Error(`Artifact size differs: ${file}`);
  const hash = crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk);
  if (hash.digest('hex') !== expected.sha256) throw new Error(`Artifact digest differs: ${file}`);
}

await check(archive, release);
const checks = artifacts.artifacts.map(item => ({ file: `models/${item.filename}`, ...item }));
const runtime = artifacts.onnx_runtime.targets[release.target];
checks.push({ file: `runtime/${runtime.filename}`, ...runtime });
checks.push({ file: 'model-manifest.json', sha256: release.model_manifest_sha256 });
for (const item of checks) await check(path.join(directory, item.file), item);
console.log(JSON.stringify({ archive_sha256: release.sha256, artifacts_verified: checks.length, baseline_commit: release.baseline_commit, release_source_commit: release.release_source_commit, release_executable_accepted: false }));
