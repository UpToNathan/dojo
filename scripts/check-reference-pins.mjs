#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { validateGithubPin, referenceIdentity, generatedReferenceDirectory } from './reference-pins.mjs';
import { DESKTOP_MCP_PROTOCOL_VERSION } from '../tools/speech/desktop-extension.mjs';

const root = path.resolve(import.meta.dirname, '..');
const digest = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
function files(directory = root) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
    if (entry.isSymbolicLink() || (entry.isDirectory() && generatedReferenceDirectory(entry.name))) return [];
    const absolute = path.join(directory, entry.name);
    return entry.isDirectory() ? files(absolute) : [path.relative(root, absolute)];
  }).sort();
}
const all = files();
const read = name => fs.readFileSync(path.join(root, name));
const locks = all.filter(name => /(?:^|\/)(?:Cargo\.lock|go\.sum|package-lock\.json|devenv\.lock|flake\.lock|lazy-lock\.json|uv\.lock|bun\.lock|pnpm-lock\.yaml|build\.zig\.zon|.*\.lock\.json)$/.test(name));
const lockedInputs = JSON.parse(read('devenv.lock'));
const errors = [];
for (const product of ['driver', 'spaces']) {
  if (JSON.parse(read(`provenance/contracts/cua-${product}.manifest.json`)).mcp_protocol_version !== DESKTOP_MCP_PROTOCOL_VERSION) {
    errors.push(`Desktop MCP protocol differs from pinned ${product} contract`);
  }
}
for (const source of JSON.parse(read('provenance/artifacts/sources.json'))) {
  const bytes = read(source.file);
  const blob = crypto.createHash('sha1').update(`blob ${bytes.length}\0`).update(bytes).digest('hex');
  if (source.commit !== '88aff7e5424e9ed45b93ded85eb4111472ca5190' || blob !== source.git_blob || digest(bytes) !== source.sha256) {
    errors.push(`Artifact lock provenance differs: ${source.file}`);
  }
}
const nix = [];
for (const [name, key] of Object.entries(lockedInputs.nodes.root.inputs)) {
  const node = lockedInputs.nodes[key];
  errors.push(...validateGithubPin(name, node, read('devenv.yaml').toString()));
  if (node.locked?.type !== 'github') { errors.push(`Nix input ${name} has no reviewed GitHub pin`); continue; }
  const { owner, repo, rev, narHash } = node.locked;
  nix.push({ name, repository: `https://github.com/${owner}/${repo}`, commit: rev, narHash });
}
const history = execFileSync('git', ['log', '--all', '--format=%H%n%B', '--grep=git-subtree-dir'], { cwd: root, encoding: 'utf8' });
const subtrees = [...history.matchAll(/git-subtree-dir: ([^\n]+)\ngit-subtree-split: ([a-f0-9]{40})/g)]
  .map(([, directory, commit]) => ({ directory, commit }));
const sources = subtrees.filter((item, index) => subtrees.findIndex(other => other.directory === item.directory) === index);
const references = [];
for (const name of all) {
  if (name.startsWith('provenance/') || name === 'scripts/check-reference-pins.mjs') continue;
  if (!/\.(?:md|json|toml|yaml|yml|nix|rs|mjs|js|sh|py|lua|zig|zon|kdl)$/.test(name)) continue;
  const bytes = read(name);
  if (bytes.length > 4_000_000) continue;
  const lines = bytes.toString().split('\n');
  for (let i = 0; i < lines.length; i++) {
    for (const match of lines[i].matchAll(/(?:https?:\/\/|github:)[^\s"'<>`\)\]\},;]+/g)) {
      const url = match[0];
      references.push({ file: name, line: i + 1, url, ...referenceIdentity(url) });
    }
  }
}
const inventory = {
  schema_version: 1,
  scope: 'Source files in this checkout outside generated directories and .delta managed worktrees. This inventory identifies unresolved references; it does not label them pinned.',
  cua: { repository: 'https://github.com/trycua/cua', commit: '88aff7e5424e9ed45b93ded85eb4111472ca5190', ledger: 'vendor/cua/ledger.json' },
  nix_inputs: nix,
  vendored_source_bases: sources,
  dependency_locks: locks.map(file => ({ file, sha256: digest(read(file)) })),
  manifests: all.filter(file => /(?:^|\/)(?:Cargo\.toml|go\.mod|package\.json|build\.zig\.zon|devenv\.yaml|devenv\.nix|home-manager\.nix)$/.test(file)).map(file => ({ file, sha256: digest(read(file)) })),
  references,
};
const output = JSON.stringify(inventory, null, 2) + '\n';
const destination = path.join(root, 'provenance/reference-inventory.json');
if (process.argv.includes('--write')) fs.writeFileSync(destination, output);
else if (!fs.existsSync(destination) || fs.readFileSync(destination, 'utf8') !== output) errors.push('Reference inventory is stale. Review changes and run scripts/check-reference-pins.mjs --write.');
const unresolved = references.filter(ref => ref.status !== 'commit-pinned');
if (process.argv.includes('--strict') && unresolved.length) errors.push(`${unresolved.length} references still require immutable identity or an explicit classification`);
console.log(JSON.stringify({ nix_inputs: nix.length, source_bases: sources.length, locks: locks.length, references: references.length, unresolved: unresolved.length, errors }, null, 2));
if (errors.length) { for (const error of errors) console.error(error); process.exitCode = 1; }
