import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { PiRuntime } from './pi-runtime.mjs';
import { meshTools, runMesh, openMeshExtension } from './mesh-extension.mjs';

const tool = name => meshTools().find(candidate => candidate.name === name);

async function fakeMesh(t, body) {
  const dir = await mkdtemp(join(tmpdir(), 'dojo-mesh-ext-'));
  t.after(() => rm(dir, { recursive: true, force: true }));
  const path = join(dir, 'dojo-mesh');
  await writeFile(path, `#!/bin/sh\nprintf '%s\\n' "$@" > "${dir}/argv"\nprintf '%s|%s' "$DOJO_MESH_ORIGIN" "$DOJO_MESH_SESSION" > "${dir}/env"\n${body}\n`);
  await chmod(path, 0o755);
  return { path, argv: async () => (await readFile(join(dir, 'argv'), 'utf8')).trimEnd().split('\n'), env: () => readFile(join(dir, 'env'), 'utf8') };
}

test('the agent has tools to look, run, copy and hand off, and none to change the network or lift the pause', () => {
  assert.deepEqual(meshTools().map(candidate => candidate.name), ['mesh_status', 'mesh_find', 'mesh_exec', 'mesh_copy', 'mesh_task']);
  assert.deepEqual(meshTools().filter(candidate => candidate.replay === 'safe').map(candidate => candidate.name), ['mesh_status', 'mesh_find']);
});

test('mesh_exec passes the program and arguments after -- with no shell of its own', () => {
  assert.deepEqual(tool('mesh_exec').command({ peer: 'beta', argv: ['ls', '-l', '$HOME; rm'] }), ['exec', '--timeout', '120', 'beta', '--', 'ls', '-l', '$HOME; rm']);
  assert.deepEqual(tool('mesh_exec').command({ peer: 'beta.example.ts.net', script: 'uptime | head -1', timeout_seconds: 5 }), ['exec', '--timeout', '5', 'beta.example.ts.net', '--', 'sh', '-c', 'uptime | head -1']);
});

test('mesh_exec refuses ambiguous or unsafe requests before running anything', () => {
  const exec = tool('mesh_exec');
  for (const args of [
    { peer: 'beta' }, { peer: 'beta', argv: ['a'], script: 'b' }, { peer: 'beta', argv: [] }, { peer: 'beta', argv: [1] },
    { peer: '-rf', argv: ['a'] }, { peer: 'a b', argv: ['a'] }, { peer: '', argv: ['a'] }, { argv: ['a'] },
    { peer: 'beta', argv: ['a'], timeout_seconds: 0 }, { peer: 'beta', argv: ['a'], timeout_seconds: 601 }, { peer: 'beta', script: '' },
  ]) assert.throws(() => exec.command(args), undefined, JSON.stringify(args));
});

test('copy, task and find build the commands the CLI expects', () => {
  assert.deepEqual(tool('mesh_copy').command({ source: './a', destination: 'beta:/tmp/a' }), ['cp', '--', './a', 'beta:/tmp/a']);
  assert.deepEqual(tool('mesh_task').command({ peer: 'beta', text: '--open the logs' }), ['task', 'beta', '--', '--open the logs']);
  assert.deepEqual(tool('mesh_find').command({ capability: 'nix-cache' }), ['find', 'nix-cache', '--json']);
  assert.throws(() => tool('mesh_find').command({ capability: '--json' }));
  assert.throws(() => tool('mesh_task').command({ peer: 'beta' }));
});

test('calls reach dojo-mesh marked as the agent, with the conversation as the session', async t => {
  const mesh = await fakeMesh(t, 'echo remote output');
  const result = await runMesh(mesh.path, ['status', '--json'], { session: 'conv-7' });
  assert.deepEqual(result, { text: 'remote output', isError: false });
  assert.equal(await mesh.env(), 'agent|conv-7');
  assert.deepEqual(await mesh.argv(), ['status', '--json']);
});

test('each exit status says what the model must do next', async t => {
  for (const [code, expected] of [[1, /reached the peer and failed/], [2, /malformed/], [3, /person has to act[\s\S]*Do not retry/], [4, /refused or could not reach[\s\S]*paused/], [9, /status 9/]]) {
    const mesh = await fakeMesh(t, `echo "the message" >&2; exit ${code}`);
    const result = await runMesh(mesh.path, ['x'], { session: 's' });
    assert.equal(result.isError, true);
    assert.match(result.text, /the message/);
    assert.match(result.text, expected, `exit ${code}`);
  }
});

test('long output is cut in the middle and a slow call is stopped', async t => {
  const long = await fakeMesh(t, 'head -c 60000 /dev/zero | tr "\\0" x');
  const clipped = await runMesh(long.path, ['x'], { session: 's' });
  assert.ok(clipped.text.length < 21000 && /characters left out/.test(clipped.text));
  const slow = await fakeMesh(t, 'exec sleep 30');
  const stopped = await runMesh(slow.path, ['x'], { session: 's', timeoutMs: 200 });
  assert.equal(stopped.isError, true);
  assert.match(stopped.text, /took too long/);
});

test('without an installed dojo-mesh the agent simply has no mesh tools', async () => {
  const runtime = { pi: { defineExtension() { throw new Error('must not build an extension'); } }, config: { meshExe: '/nonexistent/dojo-mesh' } };
  assert.equal(await openMeshExtension(runtime), undefined);
});

test('through the runtime, a model call to mesh_exec runs dojo-mesh as the agent', async t => {
  const mesh = await fakeMesh(t, 'echo ran-on-peer');
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-mesh-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider);
  faux.setResponses([fauxAssistantMessage([fauxToolCall('mesh_exec', { peer: 'beta', argv: ['uname'] })], { stopReason: 'toolUse' }), fauxAssistantMessage('done')]);
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { async append() {}, async readyView() { return '<chat>\n</chat>'; } },
    config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'test', instructions: 'fixed', desktopTools: false, meshExe: mesh.path } });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  await runtime.submit('Check beta'); await runtime.queue; await runtime.conversation.waitForIdle(runtime.context); await runtime.archived;
  assert.deepEqual(await mesh.argv(), ['exec', '--timeout', '120', 'beta', '--', 'uname']);
  assert.match(await mesh.env(), /^agent\|/);
});
