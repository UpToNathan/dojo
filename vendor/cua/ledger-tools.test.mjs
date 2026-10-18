import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve, join } from 'node:path';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
const sha = bytes => createHash('sha256').update(bytes).digest('hex');
const scriptRoot = new URL('.', import.meta.url);
function fixture(t, state = 'copied') {
  const parent = mkdtempSync(join(tmpdir(),'dojo-ledger-tool-test-'));
  t.after(()=>rmSync(parent,{recursive:true,force:true}));
  const root = join(parent,'maintained'), source = join(parent,'source');
  mkdirSync(join(root,'vendor/cua'),{recursive:true});mkdirSync(source);
  for(const name of ['refresh-ledger.mjs','verify-ledger.mjs'])writeFileSync(join(root,'vendor/cua',name),readFileSync(new URL(name,scriptRoot)));
  execFileSync('git',['init','-q',source]);
  writeFileSync(join(source,'original.rs'),'upstream bytes\n');
  execFileSync('git',['-C',source,'add','original.rs']);
  execFileSync('git',['-C',source,'-c','user.name=fixture','-c','user.email=fixture@example.invalid','commit','-qm','source']);
  const revision=execFileSync('git',['-C',source,'rev-parse','HEAD']).toString().trim();
  writeFileSync(join(root,'maintained.rs'),state==='modified'?'maintained bytes\n':'upstream bytes\n');
  const ledger={schema_version:1,owner:'fixture',upstream:{revision},units:[],files:[{path:'maintained.rs',state,
    sha256:sha(readFileSync(join(root,'maintained.rs'))),upstream_path:'original.rs',upstream_sha256:sha('upstream bytes\n')}]};
  writeFileSync(join(root,'vendor/cua/ledger.json'),JSON.stringify(ledger));
  const run=(name,...args)=>spawnSync(process.execPath,[join(root,'vendor/cua',name),'--root',root,'--source',source,...args],{encoding:'utf8'});
  return {root,source,run};
}
test('refresh refuses changed copied bytes and leaves ledger untouched',t=>{
  const f=fixture(t),ledger=readFileSync(join(f.root,'vendor/cua/ledger.json'));
  writeFileSync(join(f.root,'maintained.rs'),'unauthorized change\n');
  const result=f.run('refresh-ledger.mjs','--write');assert.notEqual(result.status,0);
  assert.match(result.stderr,/Changed copied source refused/);assert.deepEqual(readFileSync(join(f.root,'vendor/cua/ledger.json')),ledger);
});
test('declared modified patch reproduces exact bytes from pinned git objects',t=>{
  const f=fixture(t,'modified');const refresh=f.run('refresh-ledger.mjs','--write');assert.equal(refresh.status,0,refresh.stderr);
  const verify=f.run('verify-ledger.mjs');assert.equal(verify.status,0,verify.stderr);assert.match(verify.stdout,/"patches_reproduced":1/);
});
test('refresh is idempotent and dirty source working trees are ignored',t=>{
  const f=fixture(t,'modified');assert.equal(f.run('refresh-ledger.mjs','--write').status,0);
  const ledger=readFileSync(join(f.root,'vendor/cua/ledger.json'));
  writeFileSync(join(f.source,'original.rs'),'dirty source working tree\n');
  assert.equal(f.run('refresh-ledger.mjs','--write').status,0);
  assert.deepEqual(readFileSync(join(f.root,'vendor/cua/ledger.json')),ledger);
});
test('patch with a self-consistent forged checksum still fails correspondence',t=>{
  const f=fixture(t,'modified');assert.equal(f.run('refresh-ledger.mjs','--write').status,0);
  const ledgerPath=join(f.root,'vendor/cua/ledger.json');const ledger=JSON.parse(readFileSync(ledgerPath));
  const file=ledger.files.find(file=>file.path==='maintained.rs');writeFileSync(resolve(f.root,file.patch),'forged diff\n');file.patch_sha256=sha('forged diff\n');
  writeFileSync(ledgerPath,JSON.stringify(ledger));const verify=f.run('verify-ledger.mjs');assert.notEqual(verify.status,0);assert.match(verify.stderr,/Patch does not describe maintained source/);
});
test('offline verification still rejects altered local source',t=>{
  const f=fixture(t,'modified');assert.equal(f.run('refresh-ledger.mjs','--write').status,0);
  writeFileSync(join(f.root,'maintained.rs'),'changed later');const verify=spawnSync(process.execPath,[join(f.root,'vendor/cua/verify-ledger.mjs'),'--root',f.root],{encoding:'utf8'});
  assert.notEqual(verify.status,0);assert.match(verify.stderr,/Local byte mismatch/);
});
test('verification rejects dependency metadata from an older owned manifest',t=>{
  const f=fixture(t,'modified');assert.equal(f.run('refresh-ledger.mjs','--write').status,0);
  writeFileSync(join(f.root,'Cargo.toml'),'new manifest\n');
  writeFileSync(join(f.root,'vendor/cua/dependency-closure.json'),JSON.stringify({units:{fixture:{manifest:'Cargo.toml',manifest_sha256:sha('old manifest\n')}}}));
  const verify=f.run('verify-ledger.mjs');assert.notEqual(verify.status,0);assert.match(verify.stderr,/Dependency audit manifest stale/);
});
test('duplicate owned record cannot shadow an immutable copied record',t=>{
  const f=fixture(t),path=join(f.root,'vendor/cua/ledger.json');const ledger=JSON.parse(readFileSync(path));
  ledger.files.push({...ledger.files[0],state:'local'});writeFileSync(path,JSON.stringify(ledger));
  writeFileSync(join(f.root,'maintained.rs'),'changed copied source');
  const refresh=f.run('refresh-ledger.mjs','--write');assert.notEqual(refresh.status,0);assert.match(refresh.stderr,/Duplicate ledger paths/);
});
test('declared driver addition cannot hide an existing upstream file',t=>{
  const f=fixture(t),path='crates/cua-driver-core/src/host_policy.rs';
  mkdirSync(join(f.root,'vendor/cua/driver/crates/cua-driver-core/src'),{recursive:true});
  writeFileSync(join(f.root,'vendor/cua/driver',path),'maintained policy\n');
  writeFileSync(join(f.root,'vendor/cua/driver/patches.json'),JSON.stringify({new_source:[path]}));
  mkdirSync(join(f.source,'libs/cua-driver/rust/crates/cua-driver-core/src'),{recursive:true});
  writeFileSync(join(f.source,'libs/cua-driver/rust',path),'upstream policy\n');
  execFileSync('git',['-C',f.source,'add','.']);
  execFileSync('git',['-C',f.source,'-c','user.name=fixture','-c','user.email=fixture@example.invalid','commit','-qm','policy']);
  const ledgerPath=join(f.root,'vendor/cua/ledger.json'),ledger=JSON.parse(readFileSync(ledgerPath));
  ledger.upstream.revision=execFileSync('git',['-C',f.source,'rev-parse','HEAD']).toString().trim();
  writeFileSync(ledgerPath,JSON.stringify(ledger));
  const refresh=f.run('refresh-ledger.mjs','--write');
  assert.notEqual(refresh.status,0);assert.match(refresh.stderr,/Driver addition exists upstream/);
});

test('the main ledger checks whole import bytes without an upstream checkout',t=>{
  const f=fixture(t);
  const ledgerPath=join(f.root,'vendor/cua/ledger.json');const ledger=JSON.parse(readFileSync(ledgerPath));
  const imported='vendor/cua/upstream/crate/source.rs';mkdirSync(join(f.root,'vendor/cua/upstream/crate'),{recursive:true});
  writeFileSync(join(f.root,imported),'whole source\n');
  writeFileSync(join(f.root,'vendor/cua/copy-plan.json'),'{}');
  writeFileSync(join(f.root,'vendor/cua/copy-inventory.json'),JSON.stringify({revision:ledger.upstream.revision,
    files:[{path:imported,mode:'100644',sha256:sha('whole source\n')}]}));
  for(const path of ['vendor/cua/copy-plan.json','vendor/cua/copy-inventory.json'])ledger.files.push({path,state:'local',sha256:sha(readFileSync(join(f.root,path)))});
  writeFileSync(ledgerPath,JSON.stringify(ledger));
  const run=()=>spawnSync(process.execPath,[join(f.root,'vendor/cua/verify-ledger.mjs'),'--root',f.root],{encoding:'utf8'});
  const initial=run();assert.equal(initial.status,0,initial.stderr);assert.match(initial.stdout,/"whole_import_files":1/);
  writeFileSync(join(f.root,imported),'changed whole source\n');
  const changed=run();assert.notEqual(changed.status,0);assert.match(changed.stderr,/Whole import byte mismatch/);
});

// A source whose upstream file changed after the first commit; the ledger sits at the second.
function twoRevisions(t) {
  const f = fixture(t), first = JSON.parse(readFileSync(join(f.root, 'vendor/cua/ledger.json'))).upstream.revision;
  writeFileSync(join(f.source, 'original.rs'), 'newer upstream bytes\n');
  execFileSync('git', ['-C', f.source, '-c', 'user.name=fixture', '-c', 'user.email=fixture@example.invalid', 'commit', '-qam', 'newer']);
  const second = execFileSync('git', ['-C', f.source, 'rev-parse', 'HEAD']).toString().trim();
  const ledgerPath = join(f.root, 'vendor/cua/ledger.json'), ledger = JSON.parse(readFileSync(ledgerPath));
  ledger.upstream.revision = second;
  return {...f, first, second, ledger, save: () => writeFileSync(ledgerPath, JSON.stringify(ledger))};
}
test('a record pinned to an older upstream revision verifies while the ledger sits at a newer one', t => {
  const f = twoRevisions(t);
  f.ledger.files[0].upstream_revision = f.first;
  f.ledger.source_evidence = [{path: 'original.rs', sha256: sha('upstream bytes\n'), upstream_revision: f.first},
    {path: 'original.rs', sha256: sha('newer upstream bytes\n')}];
  f.save();
  const verify = f.run('verify-ledger.mjs'); assert.equal(verify.status, 0, verify.stderr);
  assert.equal(f.run('refresh-ledger.mjs').status, 0);
});
test('a wrong hash for an older revision fails', t => {
  const f = twoRevisions(t);
  f.ledger.files[0].upstream_revision = f.first; f.ledger.files[0].upstream_sha256 = sha('newer upstream bytes\n'); f.save();
  const verify = f.run('verify-ledger.mjs'); assert.notEqual(verify.status, 0); assert.match(verify.stderr, /Pinned source mismatch/);
  const refresh = f.run('refresh-ledger.mjs'); assert.notEqual(refresh.status, 0); assert.match(refresh.stderr, /Changed copied source refused/);
  f.ledger.files[0].upstream_sha256 = sha('upstream bytes\n'); f.ledger.source_evidence = [{path: 'original.rs', sha256: sha('newer upstream bytes\n'), upstream_revision: f.first}]; f.save();
  const evidence = f.run('verify-ledger.mjs'); assert.notEqual(evidence.status, 0); assert.match(evidence.stderr, /Translation source mismatch/);
});
test('records without upstream_revision still use the ledger revision', t => {
  const f = twoRevisions(t);
  f.ledger.files[0].upstream_sha256 = sha('upstream bytes\n'); f.save();
  const stale = f.run('verify-ledger.mjs'); assert.notEqual(stale.status, 0); assert.match(stale.stderr, /Pinned source mismatch/);
  f.ledger.files[0].upstream_sha256 = sha('newer upstream bytes\n');
  writeFileSync(join(f.root, 'maintained.rs'), 'newer upstream bytes\n'); f.ledger.files[0].sha256 = sha('newer upstream bytes\n'); f.save();
  const current = f.run('verify-ledger.mjs'); assert.equal(current.status, 0, current.stderr);
});
test('upstream_revision must be a full commit id', t => {
  const f = twoRevisions(t);
  f.ledger.files[0].upstream_revision = 'main'; f.save();
  const verify = f.run('verify-ledger.mjs'); assert.notEqual(verify.status, 0); assert.match(verify.stderr, /Invalid upstream revision/);
  assert.match(f.run('refresh-ledger.mjs').stderr, /Invalid upstream revision/);
});
