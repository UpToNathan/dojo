#!/usr/bin/env node
// Uses the same nested-compositor isolation as check-browser-control-live.mjs.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { connect, createServer } from 'node:net';
import { mkdtemp, readFile, writeFile, open } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { DesktopMcp } from './desktop-extension.mjs';

const gtk = process.env.DOJO_TEST_GTK_LIBRARY;
assert(gtk?.startsWith('/nix/store/') && gtk.endsWith('/lib/libgtk-4.so.1'), 'Set DOJO_TEST_GTK_LIBRARY to the immutable GTK4 library');
const root = process.cwd();
const evidence = await mkdtemp(join(tmpdir(), 'dojo-accessibility-live.'));
console.log(`Accessibility acceptance evidence: ${evidence}`);
const children = [], logs = [];
let mcp, staleMcp, proxy;
const request = (path, value) => new Promise((resolve, reject) => {
  const socket = connect(path); let text = '';
  socket.setTimeout(3000, () => socket.destroy(new Error('Niri IPC deadline')));
  socket.on('error', reject);
  socket.on('connect', () => socket.write(JSON.stringify(value) + '\n'));
  socket.on('data', bytes => {
    text += bytes; if (!text.includes('\n')) return;
    socket.end();
    try { const result = JSON.parse(text.split('\n')[0]); if (result.Err) reject(new Error(result.Err)); else resolve(result.Ok); }
    catch (error) { reject(error); }
  });
});
const wait = async (label, check) => {
  const deadline = Date.now() + 20000;
  while (Date.now() < deadline) {
    const value = await check(); if (value) return value;
    await new Promise(resolve => setTimeout(resolve, 50));
  }
  throw new Error(`Timed out: ${label}`);
};
const launch = async (command, args, env, label) => {
  const log = await open(join(evidence, `${label}.log`), 'w'); logs.push(log);
  const child = spawn(command, args, { env, detached: true, stdio: ['ignore', log.fd, log.fd] });
  child.on('error', error => { child.failure = error; }); children.push(child); return child;
};
const active = workspaces => workspaces.filter(w => w.is_active).map(({ output, id }) => ({ output, id })).sort((a, b) => a.output.localeCompare(b.output));
const value = result => {
  assert.equal(result.isError ?? false, false, JSON.stringify(result));
  return result.structuredContent ?? JSON.parse(result.content.find(block => block.type === 'text').text);
};
try {
  const original = (await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces;
  const focused = original.find(w => w.is_focused);
  const test = original.find(w => w.name === 'dojo-tests' && !w.is_active)
    ?? original.find(w => !w.is_active && !w.name && w.active_window_id === null && w.output === focused?.output);
  assert(test, 'An inactive empty workspace is required');
  if (test.name !== 'dojo-tests') await request(process.env.NIRI_SOCKET, { Action: { SetWorkspaceName: { name: 'dojo-tests', workspace: { Id: test.id } } } });
  const env = { ...process.env, DOJO_AGENT_INPUT: '1', RUST_LOG: 'niri=info' };
  const niri = await launch(process.env.NIRI_BINARY ?? join(root, 'vendor/niri/target/debug/niri'), ['--config', join(root, 'tools/agent-input/nested.kdl')], env, 'niri');
  const nested = await wait('nested compositor', async () => {
    if (niri.exitCode !== null || niri.failure) throw new Error('Nested compositor failed');
    const log = await readFile(join(evidence, 'niri.log'), 'utf8');
    const display = [...log.matchAll(/listening on Wayland socket: (\S+)/g)].at(-1)?.[1];
    const socket = [...log.matchAll(/IPC listening on: (\S+)/g)].at(-1)?.[1];
    return display && socket ? { display, socket } : undefined;
  });
  const host = await wait('nested host window', async () => (await request(process.env.NIRI_SOCKET, 'Windows')).Windows.find(w => w.pid === niri.pid));
  await request(process.env.NIRI_SOCKET, { Action: { MoveWindowToWorkspace: { window_id: host.id, reference: { Id: test.id }, focus: false } } });
  assert.deepEqual(active((await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces), active(original));
  const fixture = join(evidence, 'gtk-fixture');
  execFileSync('cc', [join(root, 'tools/agent-input/gtk-accessibility-fixture.c'), '-o', fixture, '-ldl']);
  const nestedEnv = { ...env, WAYLAND_DISPLAY: nested.display, NIRI_SOCKET: nested.socket, GTK_A11Y: 'atspi', GDK_BACKEND: 'wayland' };
  const app = await launch(fixture, [gtk], nestedEnv, 'gtk');
  const windows = await wait('two same-process windows', async () => {
    if (app.exitCode !== null || app.failure) throw new Error('GTK fixture failed');
    const windows = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === app.pid);
    return windows.length === 2 ? windows : undefined;
  });
  assert.equal(windows[0].title, windows[1].title);
  const targets = await wait('two GTK surface annotations', async () => {
    const targets = await Promise.all(windows.map(async w => (await request(nested.socket, { AccessibilityTarget: { window_id: w.id } })).AccessibilityTarget));
    return targets.every(t => t.endpoint) ? targets : undefined;
  });
  assert.notEqual(targets[0].endpoint.object_path, targets[1].endpoint.object_path);
  mcp = await DesktopMcp.open(root, { env: nestedEnv });
  const call = (name, args) => mcp.call('tools/call', { name, arguments: args });
  const observations = [];
  for (const window of windows) {
    const tree = value(await call('desktop_accessibility', { window_id: window.id }));
    await writeFile(join(evidence, `window-${window.id}.json`), JSON.stringify(tree, null, 2));
    assert.equal(tree.scope, 'window'); assert.equal(tree.window_scoped, true); assert.equal(tree.actionable, false);
    assert.equal(tree.available, true); assert.equal(tree.requested_window_id, window.id);
    assert(!JSON.stringify(tree).includes('fixture-sensitive-password'));
    const alpha = tree.nodes.some(n => n.name === 'Alpha Save');
    const beta = tree.nodes.some(n => n.name === 'Beta Save');
    assert.notEqual(alpha, beta, JSON.stringify(tree));
    const label = alpha ? 'Alpha Save' : 'Beta Save';
    const other = alpha ? 'Beta Save' : 'Alpha Save';
    const complete = tree.semantic_complete;
    const verified = value(await call('desktop_verify', { pid: app.pid, window_id: window.id, timeout_ms: 5000,
      expect: [{ element: { selector: { role: 'button', label_contains: label }, exists: true } }] }));
    assert.equal(verified.status, 'satisfied', JSON.stringify(verified));
    const state = value(await call('desktop_verify', { pid: app.pid, window_id: window.id, timeout_ms: 5000,
      expect: [{ element: { selector: { role: 'button', label_contains: label }, enabled: true } }] }));
    assert.equal(state.status, complete ? 'satisfied' : 'unknown', JSON.stringify(state));
    const sibling = value(await call('desktop_verify', { pid: app.pid, window_id: window.id, timeout_ms: 5000,
      expect: [{ element: { selector: { role: 'button', label_contains: other }, exists: true } }] }));
    assert.equal(sibling.status, complete ? 'unsatisfied' : 'unknown', JSON.stringify(sibling));
    observations.push({ window_id: window.id, tree, verified, state, sibling });
  }
  const proxyPath = join(evidence, 'stale-native.sock');
  proxy = createServer(socket => {
    let buffered = '', targetsRead = 0, queue = Promise.resolve();
    socket.on('error', () => {});
    socket.on('data', bytes => {
      buffered += bytes;
      while (buffered.includes('\n')) {
        const boundary = buffered.indexOf('\n');
        const line = buffered.slice(0, boundary); buffered = buffered.slice(boundary + 1);
        queue = queue.then(async () => {
          const input = JSON.parse(line);
          const result = await request(nested.socket, input);
          if (input.AccessibilityTarget && ++targetsRead === 2) result.AccessibilityTarget.generation++;
          socket.write(JSON.stringify({ Ok: result }) + '\n');
        }).catch(() => socket.destroy());
      }
    });
  });
  await new Promise((resolve, reject) => { proxy.once('error', reject); proxy.listen(proxyPath, resolve); });
  staleMcp = await DesktopMcp.open(root, { env: { ...nestedEnv, NIRI_SOCKET: proxyPath } });
  const stale = await staleMcp.call('tools/call', { name: 'desktop_accessibility', arguments: { window_id: windows[0].id } });
  assert.equal(stale.isError, true, JSON.stringify(stale));
  assert.match(JSON.stringify(stale), /target changed during observation/);
  staleMcp.close(); staleMcp = undefined;
  app.kill('SIGTERM');
  await wait('fixture window retirement', async () => !(await request(nested.socket, 'Windows')).Windows.some(w => w.pid === app.pid));
  assert.equal((await call('desktop_accessibility', { window_id: windows[0].id })).isError, true);
  const final = (await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces;
  assert.deepEqual(active(final), active(original));
  assert.equal(final.find(w => w.id === test.id).is_active, false);
  await writeFile(join(evidence, 'acceptance.json'), JSON.stringify({ passed: true, gtk, gtk_sha256: createHash('sha256').update(await readFile(gtk)).digest('hex'),
    targets, observations, stale_generation_refused: true, initial_active: active(original), final_active: active(final), retired_target_refused: true }, null, 2));
} finally {
  staleMcp?.close(); proxy?.close();
  mcp?.close();
  for (const child of children.reverse()) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  await Promise.all(logs.map(log => log.close()));
}
