#!/usr/bin/env node
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const root = path.resolve(import.meta.dirname, '..');
const sha256 = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const contracts = ['driver', 'spaces'].map(product => {
  const file = `provenance/contracts/cua-${product}.manifest.json`;
  const bytes = fs.readFileSync(path.join(root, file));
  const manifest = JSON.parse(bytes);
  return {
    product,
    file,
    sha256: sha256(bytes),
    contract_version: manifest.contract_version,
    mcp_protocol_version: manifest.mcp_protocol_version,
    tools: manifest.tools.map(tool => ({
      name: tool.name,
      contract_sha256: sha256(JSON.stringify(tool)),
      status: 'unverified',
      evidence: [],
    })),
  };
});
const destination = path.join(root, 'provenance/cua-parity.json');
const existing = fs.existsSync(destination) ? JSON.parse(fs.readFileSync(destination)) : null;
if (existing) {
  for (const contract of contracts) {
    const prior = existing.contracts.find(item => item.product === contract.product);
    if (!prior || prior.sha256 !== contract.sha256) throw new Error(`Pinned ${contract.product} contract changed; explicit baseline migration required`);
    for (const tool of contract.tools) {
      const old = prior.tools.find(item => item.name === tool.name);
      if (!old || old.contract_sha256 !== tool.contract_sha256) throw new Error(`Pinned tool differs: ${contract.product}/${tool.name}`);
      tool.status = old.status;
      tool.evidence = old.evidence;
      if (!['unverified', 'partial', 'verified', 'blocked'].includes(tool.status)) throw new Error(`Invalid parity state: ${tool.name}`);
      if (tool.status === 'verified' && !tool.evidence.length) throw new Error(`Verified tool has no evidence: ${tool.name}`);
      for (const evidence of tool.evidence) {
        const absolute = path.resolve(root, evidence.file);
        if (!absolute.startsWith(`${root}/`) || sha256(fs.readFileSync(absolute)) !== evidence.sha256) throw new Error(`Parity evidence differs: ${tool.name}`);
      }
    }
  }
}
const index = { schema_version: 1, upstream_commit: 'afed1fcbba64acadcc154b8d5d377549ae62bc7e', contracts };
const bytes = JSON.stringify(index, null, 2) + '\n';
if (process.argv.includes('--write')) fs.writeFileSync(destination, bytes);
else if (!existing || fs.readFileSync(destination, 'utf8') !== bytes) throw new Error('Parity index is stale; review baseline and run with --write');
const tools = contracts.flatMap(item => item.tools);
const remaining = tools.filter(tool => tool.status !== 'verified');
console.log(JSON.stringify({ tools: tools.length, verified: tools.length - remaining.length, remaining: remaining.length }));
if (process.argv.includes('--strict') && remaining.length) process.exitCode = 1;
