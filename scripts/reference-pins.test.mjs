import test from 'node:test';
import assert from 'node:assert/strict';
import { validateGithubPin, referenceIdentity, generatedReferenceDirectory } from './reference-pins.mjs';

const rev = 'c2f38fe7f9e04d9aadd354d380f2bd40531d9737';
test('the inventory covers this checkout rather than generated copies or build trees', () => {
  assert.equal(generatedReferenceDirectory('.delta'), true);
  assert.equal(generatedReferenceDirectory('target'), true);
  assert.equal(generatedReferenceDirectory('.scratch'), false);
  assert.equal(generatedReferenceDirectory('vendor'), false);
  assert.equal(generatedReferenceDirectory('crates'), false);
});
const node = { locked: { type: 'github', owner: 'cachix', repo: 'devenv-nixpkgs', rev, narHash: `sha256-${'A'.repeat(43)}=` }, original: { rev } };

test('locked bytes alone do not pin a moving declared input', () => {
  assert.equal(validateGithubPin('nixpkgs', { ...node, original: { ref: 'rolling' } }, `  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/${rev}`).length, 1);
});
test('input URL must match its own heading and full repository name', () => {
  assert.deepEqual(validateGithubPin('nixpkgs', node, `  nixpkgs:\n    url: github:cachix/devenv-nixpkgs/${rev}`), []);
  assert.equal(validateGithubPin('nixpkgs', node, `  nixpkgs:\n    url: github:cachix/devenv/${rev}-nixpkgs/rolling`).length, 1);
  assert.equal(validateGithubPin('nixpkgs', node, `  other:\n    url: github:cachix/devenv-nixpkgs/${rev}`).length, 1);
});
test('moving branches and version tags never become commit pins', () => {
  assert.deepEqual(referenceIdentity('https://github.com/a/b/blob/main/file.rs'), { identity: null, status: 'moving-reference' });
  assert.deepEqual(referenceIdentity('https://github.com/a/b/archive/v1.0.tar.gz'), { identity: null, status: 'requires-classification' });
  assert.deepEqual(referenceIdentity(`https://github.com/a/b/archive/${rev}.tar.gz`), { identity: { git_commit: rev }, status: 'commit-pinned' });
  assert.deepEqual(referenceIdentity(`github:cachix/devenv/${rev}?dir=src/modules`), { identity: { git_commit: rev }, status: 'commit-pinned' });
});

test('a hash in an arbitrary URL or binary release path is not a source pin',()=>{
  assert.deepEqual(referenceIdentity(`https://example.com/${rev}/binary`),{identity:null,status:'requires-classification'});
  assert.deepEqual(referenceIdentity(`https://github.com/a/b/releases/download/${rev}/binary`),{identity:null,status:'requires-classification'});
  assert.deepEqual(referenceIdentity(`https://raw.githubusercontent.com/a/b/${rev}/source.rs`),{identity:{git_commit:rev},status:'commit-pinned'});
});
