import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import {execFileSync, spawnSync} from 'node:child_process';

const copier = path.join(import.meta.dirname, 'copy-upstream.mjs');
function fixture(t, link = 'src/value.rs') {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'dojo-copy-test-'));
  t.after(() => fs.rmSync(root, {recursive:true,force:true}));
  const source = path.join(root, 'source');
  fs.mkdirSync(path.join(source, 'crate/src'), {recursive:true});
  fs.writeFileSync(path.join(source, 'crate/Cargo.toml'), '[workspace]\nresolver = "2"\nmembers = ["old"]\n\n[workspace.package]\nversion = "1.2.3"\n');
  fs.writeFileSync(path.join(source, 'crate/src/value.rs'), 'pub const VALUE: u32 = 7;\n');
  fs.symlinkSync(link, path.join(source, 'crate/value'));
  const git = (...args) => execFileSync('git', ['-C',source,...args], {encoding:'utf8'}).trim();
  git('init','-q'); git('add','.');
  git('-c','user.name=Fixture','-c','user.email=fixture@example.invalid','commit','-qm','baseline');
  const revision = git('rev-parse','HEAD');
  fs.mkdirSync(path.join(root, 'vendor/cua'), {recursive:true});
  fs.writeFileSync(path.join(root,'vendor/cua/copy-plan.json'), JSON.stringify({revision,destination:'vendor/cua/upstream',
    trees:['crate'],files:[],workspaces:[{file:'crate/Cargo.toml',members:['src']}]}));
  const run = (...args) => spawnSync(process.execPath,[copier,source,'--root',root,...args],{encoding:'utf8'});
  return {root,source,run,git};
}
test('whole copies preserve source and safe links; membership is the only adaptation', t => {
  const f=fixture(t);
  assert.equal(f.run().status,0);
  const inventory=JSON.parse(fs.readFileSync(path.join(f.root,'vendor/cua/copy-inventory.json')));
  assert.equal(inventory.files.filter(file=>file.state==='copied').length,2);
  assert.equal(inventory.files.filter(file=>file.state==='modified').length,1);
  assert.equal(fs.readlinkSync(path.join(f.root,'vendor/cua/upstream/crate/value')),'src/value.rs');
  assert.equal(fs.readFileSync(path.join(f.root,'vendor/cua/upstream/crate/src/value.rs'),'utf8'),'pub const VALUE: u32 = 7;\n');
  assert.equal(f.run('--verify').status,0);
  assert.equal(f.run().status,0);
});
test('dirty upstream working files cannot redefine committed source', t => {
  const f=fixture(t);
  fs.writeFileSync(path.join(f.source,'crate/src/value.rs'),'dirty\n');
  assert.equal(f.run().status,0);
  assert.equal(fs.readFileSync(path.join(f.root,'vendor/cua/upstream/crate/src/value.rs'),'utf8'),'pub const VALUE: u32 = 7;\n');
});
test('maintained edits fail verification and are never overwritten by a rerun', t => {
  const f=fixture(t); assert.equal(f.run().status,0);
  const target=path.join(f.root,'vendor/cua/upstream/crate/src/value.rs');
  fs.writeFileSync(target,'maintained change\n');
  assert.notEqual(f.run('--verify').status,0);
  assert.notEqual(f.run().status,0);
  assert.equal(fs.readFileSync(target,'utf8'),'maintained change\n');
});
test('links outside the selected source fail before writing an import', t => {
  const f=fixture(t,'../../outside');
  assert.notEqual(f.run().status,0);
  assert.equal(fs.existsSync(path.join(f.root,'vendor/cua/upstream')),false);
});
test('an altered copy inventory is refused', t => {
  const f=fixture(t); assert.equal(f.run().status,0);
  fs.writeFileSync(path.join(f.root,'vendor/cua/copy-inventory.json'),'{}\n');
  assert.notEqual(f.run('--verify').status,0);
});

test('a pinned fork patch reproduces changed upstream and explicit new source', t => {
  const f=fixture(t);
  const crypto=process.getBuiltinModule('node:crypto');
  const sha=value=>crypto.createHash('sha256').update(value).digest('hex');
  const patch='diff --git a/crate/src/value.rs b/crate/src/value.rs\n--- a/crate/src/value.rs\n+++ b/crate/src/value.rs\n@@ -1 +1 @@\n-pub const VALUE: u32 = 7;\n+pub const VALUE: u32 = 8;\ndiff --git a/crate/src/niri.rs b/crate/src/niri.rs\nnew file mode 100644\n--- /dev/null\n+++ b/crate/src/niri.rs\n@@ -0,0 +1 @@\n+pub const NIRI: bool = true;\n';
  const metadata=path.join(f.root,'vendor/cua/metadata');fs.mkdirSync(metadata);
  fs.writeFileSync(path.join(metadata,'niri.patch'),patch);
  const planPath=path.join(f.root,'vendor/cua/copy-plan.json');
  const plan=JSON.parse(fs.readFileSync(planPath));
  plan.patches=[{metadata:'vendor/cua/metadata/niri.patch',sha256:sha(patch),files:[
    {path:'crate/src/value.rs',sha256:sha('pub const VALUE: u32 = 8;\n')},
    {path:'crate/src/niri.rs',added:true,sha256:sha('pub const NIRI: bool = true;\n')},
  ]}];
  fs.writeFileSync(planPath,JSON.stringify(plan));
  const result=f.run();assert.equal(result.status,0,result.stderr);
  assert.equal(f.run('--verify').status,0);
  const inventory=JSON.parse(fs.readFileSync(path.join(f.root,'vendor/cua/copy-inventory.json')));
  assert.equal(inventory.files.find(file=>file.upstream_path==='crate/src/niri.rs').state,'local-adaptation');
  assert.equal(inventory.files.find(file=>file.upstream_path==='crate/src/value.rs').state,'modified');
  fs.writeFileSync(path.join(metadata,'niri.patch'),patch.replace('u32 = 8','u32 = 9'));
  assert.notEqual(f.run('--verify').status,0);
});

test('verification refuses an executable mode change', t => {
  const f=fixture(t);assert.equal(f.run().status,0);
  fs.chmodSync(path.join(f.root,'vendor/cua/upstream/crate/src/value.rs'),0o755);
  const result=f.run('--verify');
  assert.notEqual(result.status,0);assert.match(result.stderr,/executable mode differs/);
});
