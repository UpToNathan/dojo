#!/usr/bin/env node
// Resolve locked Cargo metadata and hash available third-party license evidence.
// This reports declarations and missing evidence, never license approval.
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, readdirSync, existsSync, lstatSync, realpathSync } from 'node:fs';
import { dirname, resolve, relative, isAbsolute, basename } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
let write = false;
for (const arg of process.argv.slice(2)) { if (arg === '--write') write = true; else throw new Error('Usage: audit-dependencies.mjs [--write]; run through devenv'); }
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const manifests = [
  ['dojo-root','Cargo.toml'], ['niri-compositor','vendor/niri/Cargo.toml'], ['dojo-cua-primitives','crates/dojo-cua-primitives/Cargo.toml'],
  ['cua-cursor-motion','vendor/cua/crates/cua-cursor-motion/Cargo.toml'],
  ['cua-spaces-transcript','vendor/cua/crates/cua-spaces-transcript/Cargo.toml'],
  ['cua-driver-private','vendor/cua/driver/Cargo.toml'],
  ['cua-driver-whole','vendor/cua/upstream/libs/cua-driver/rust/Cargo.toml'],
  ['cua-sdk-whole','vendor/cua/upstream/libs/cua/Cargo.toml'],
  ...['computer','computer-models','computer-verification','capture-wayland','accessibility','agent-setup','icon-cache','computer-recording','external-admission','cua-worker','native-seat','browser-control']
    .map(name => [`dojo-${name}`, `crates/dojo-${name}/Cargo.toml`]),
];
function metadata(manifest) {
  return JSON.parse(execFileSync('cargo', ['metadata','--offline','--locked','--format-version','1',
    '--filter-platform','x86_64-unknown-linux-gnu','--no-default-features','--manifest-path',manifest],
    { cwd: root, maxBuffer: 96 * 1024 * 1024, stdio: ['ignore','pipe','pipe'] }));
}
function canonicalId(id) { return id.replace(`path+file://${root}`, 'path+repo://'); }
function normalBuildNames(manifest) {
  const tree = execFileSync('cargo', ['tree','--offline','--locked','--no-default-features','--manifest-path',manifest,
    '--target','x86_64-unknown-linux-gnu','--edges','normal,build','--prefix','none','--format','{p}','--no-dedupe'],
    {cwd:root,maxBuffer:96*1024*1024,stdio:['ignore','pipe','pipe']}).toString();
  return new Set(tree.split('\n').flatMap(line=>{const match=line.match(/^(\S+) v([^\s]+)/);return match?[`${match[1]}@${match[2]}`]:[];}));
}
function manifestLabel(path) {
  const rel = relative(root, path);
  if (!rel.startsWith('..') && !isAbsolute(rel)) return rel;
  return basename(dirname(path)) + '/Cargo.toml';
}
function licenseEvidence(pkg) {
  const directory = dirname(pkg.manifest_path), candidates = new Map();
  if (pkg.license_file) candidates.set(resolve(directory,pkg.license_file), pkg.license_file);
  for (let depth = 0, dir = directory; depth < 6; depth++, dir = dirname(dir)) {
    // Never scan general registry/git cache parent directories or the user's home.
    if (depth > 0 && (dir.includes('/registry/src/') || dir.includes('/git/checkouts/'))) break;
    for (const file of readdirSync(dir)) if (/^(?:.*-)?(license|licence|copying|notice|copyright)([._-].*)?$/i.test(file)) {
      candidates.set(resolve(dir,file), relative(directory,resolve(dir,file)));
    }
    if (dir === root || dir === dirname(dir)) break;
  }
  const evidence = [];
  for (const [path,label] of candidates) {
    if (!existsSync(path) || !lstatSync(path).isFile() || lstatSync(path).isSymbolicLink()) continue;
    const stats = lstatSync(path);
    if (stats.size > 2 * 1024 * 1024) throw new Error(`License evidence exceeds 2 MiB: ${pkg.name}`);
    evidence.push({ path: label, sha256: sha(readFileSync(path)), bytes: stats.size });
  }
  return evidence.sort((a,b)=>a.path < b.path ? -1 : a.path > b.path ? 1 : 0);
}

function copyrightEvidence(pkg) {
  const directory=dirname(pkg.manifest_path), evidence=[];
  for(const name of ['README.md','README','README.rst','Cargo.toml.orig','src/lib.rs','src/main.rs']) {
    const path=resolve(directory,name);
    if(!existsSync(path)||!lstatSync(path).isFile()||lstatSync(path).isSymbolicLink()||lstatSync(path).size>1024*1024)continue;
    const bytes=readFileSync(path), lines=bytes.toString('utf8').split('\n');
    const matches=lines.flatMap((line,index)=>/copyright|SPDX-License-Identifier/i.test(line)?[{line:index+1,text:line.trim().slice(0,500)}]:[]).slice(0,10);
    if(matches.length)evidence.push({path:name,sha256:sha(bytes),matches});
  }
  return evidence;
}

const closure = { schema_version: 2, platform: 'x86_64-unknown-linux-gnu',
  command: 'devenv shell -- node vendor/cua/audit-dependencies.mjs --write',
  scope: 'Locked Linux metadata for root and each maintained CUA/adaptation crate; metadata dependency declarations include target/dev/build edges, resolved nodes are filtered to Linux. No source or artifact downloads.',
  features: 'All manifests use --no-default-features. Root explicit dependency features still apply. Optional feature closure beyond these manifests is not claimed.',
  units: {}, third_party_audit: {} };
const all = new Map(), unavailable = [];
for (const [unit,manifest] of manifests) {
  if (!existsSync(resolve(root,manifest))) { unavailable.push({ unit, manifest, reason: 'manifest missing' }); continue; }
  let data;
  try { data = metadata(manifest); } catch (error) {
    throw new Error(`Locked offline metadata failed for ${manifest}: ${error.stderr?.toString().slice(0,1000) ?? error.message}`);
  }
  const activeNames = normalBuildNames(manifest);
  const packages = data.packages.map(pkg => {
    const record = { id: canonicalId(pkg.id), name: pkg.name, version: pkg.version, source: pkg.source,
      local_manifest: pkg.source ? null : manifestLabel(pkg.manifest_path),
      manifest_sha256: sha(readFileSync(pkg.manifest_path)), license: pkg.license,
      license_file: pkg.license_file, rust_version: pkg.rust_version, edition: pkg.edition,
      license_evidence: licenseEvidence(pkg),
      source_copyright_evidence: copyrightEvidence(pkg),
      normal_build_active: activeNames.has(`${pkg.name}@${pkg.version}`),
      dependencies: pkg.dependencies.map(dep => ({ name: dep.name, req: dep.req, kind: dep.kind,
        target: dep.target, optional: dep.optional, default_features: dep.uses_default_features,
        features: dep.features, rename: dep.rename })) };
    if (!all.has(pkg.id)) all.set(pkg.id,{ ...record, units: [], normal_build_units: [] });
    all.get(pkg.id).units.push(unit);
    if(record.normal_build_active)all.get(pkg.id).normal_build_units.push(unit);
    return record;
  });
  const lock = resolve(root,manifest,'..','Cargo.lock');
  closure.units[unit] = { manifest, manifest_sha256: sha(readFileSync(resolve(root,manifest))),
    lockfile_sha256: existsSync(lock) ? sha(readFileSync(lock)) : null,
    packages, resolved_graph: data.resolve?.nodes.map(node => ({ id: canonicalId(node.id), features: node.features,
      dependencies: node.deps.map(dep=>({ name: dep.name, pkg: canonicalId(dep.pkg), kinds: dep.dep_kinds })) })) ?? [],
    root_ids: data.workspace_members.map(canonicalId),
    normal_build_packages: packages.filter(pkg=>pkg.normal_build_active).map(pkg=>pkg.id),
    normal_build_scope: 'cargo tree --target x86_64-unknown-linux-gnu --edges normal,build --no-default-features --offline --locked; excludes dev edges from metadata closure'  };
}
const packages = [...all.values()].sort((a,b)=>`${a.name}@${a.version}` < `${b.name}@${b.version}` ? -1 : `${a.name}@${a.version}` > `${b.name}@${b.version}` ? 1 : 0);
const unknown = packages.filter(pkg=>!pkg.license).map(pkg=>({ name:pkg.name,version:pkg.version,source:pkg.source,
  local_manifest:pkg.local_manifest,license_file:pkg.license_file,license_evidence:pkg.license_evidence,units:pkg.units,normal_build_units:pkg.normal_build_units,
  reason:'No declared SPDX expression. Available license files require review; none are inferred or approved.' }));
const missingEvidence = packages.filter(pkg=>!pkg.license_evidence.length).map(pkg=>({name:pkg.name,version:pkg.version,
  source:pkg.source,license:pkg.license,units:pkg.units,normal_build_units:pkg.normal_build_units,source_copyright_evidence:pkg.source_copyright_evidence}));
const restricted = packages.filter(pkg=>pkg.license && /AGPL|GPL|LGPL|FSL|SSPL|BUSL|MPL|CDDL|OpenSSL|LicenseRef|Unlicense/i.test(pkg.license))
  .map(pkg=>({name:pkg.name,version:pkg.version,source:pkg.source,license:pkg.license,units:pkg.units,normal_build_units:pkg.normal_build_units,
    reason:'Declaration includes a license or alternative needing distribution/use review. No approval inferred.'}));
closure.third_party_audit = { audited_packages:packages.length,registry_or_git_packages:packages.filter(p=>p.source).length,
  unknown_declared_licenses:unknown,missing_local_license_evidence:missingEvidence,
  active_root_missing_license_evidence:missingEvidence.filter(p=>p.normal_build_units.includes('dojo-root')),
  active_root_unknown_licenses:unknown.filter(p=>p.normal_build_units.includes('dojo-root')),expressions_requiring_review:restricted,
  declared_expression_counts: Object.fromEntries([...new Set(packages.map(p=>p.license??'<unknown>'))].sort().map(license=>
    [license,packages.filter(p=>(p.license??'<unknown>')===license).length])),
  all_packages: packages.map(({dependencies,...pkg})=>pkg),
  limitations:['License expressions are declarations, not legal permission decisions.',
    'License/notice hashes prove which local texts were inspected; shipping notice assembly and license compatibility decisions are not performed.',
    'Native system libraries, model weights, datasets and optional excluded feature artifacts require separate audits.'],
  unavailable_manifests:unavailable };
if(write)writeFileSync(resolve(root,'vendor/cua/dependency-closure.json'),JSON.stringify(closure,null,2)+'\n');
console.log(JSON.stringify({written:write,units:Object.keys(closure.units).length,packages:packages.length,
  registry_or_git:closure.third_party_audit.registry_or_git_packages,unknown_licenses:unknown.map(p=>`${p.name}@${p.version}`),
  missing_license_files:missingEvidence.length,expressions_needing_review:restricted.map(p=>`${p.name}@${p.version}: ${p.license}`)}));
