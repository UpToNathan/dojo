#!/usr/bin/env node
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createServer, connect } from 'node:net';
import { createServer as httpServer } from 'node:http';
import { mkdtemp, mkdir, readFile, writeFile, open } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { DesktopMcp } from './desktop-extension.mjs';

const packagePath = process.env.DOJO_TEST_BROWSER_PACKAGE;
const broker = process.env.DOJO_BROWSER_CONTROL_BROKER_EXE;
if (!packagePath || !broker?.startsWith('/nix/store/')) throw new Error('Set DOJO_TEST_BROWSER_PACKAGE and the exact immutable DOJO_BROWSER_CONTROL_BROKER_EXE');
const evidence = await mkdtemp(join(tmpdir(), 'dojo-browser-live.'));
console.log(`Browser acceptance evidence: ${evidence}`);
const children = [];
const logs = [];
let mcp;
const wait = async (label, check, timeout = 15000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const value = await check(); if (value) return value; await new Promise(resolve => setTimeout(resolve, 50)); }
  throw new Error(`Timed out: ${label}`);
};
const request = (path, value) => new Promise((resolve, reject) => {
  const socket = connect(path); let text = '';
  socket.setTimeout(3000, () => socket.destroy(new Error('Niri IPC deadline')));
  socket.on('error', reject);
  socket.on('connect', () => socket.write(JSON.stringify(value) + '\n'));
  socket.on('data', bytes => { text += bytes; if (!text.includes('\n')) return; socket.end(); try { const result = JSON.parse(text.split('\n')[0]); if (result.Err) reject(new Error(result.Err)); else resolve(result.Ok); } catch (error) { reject(error); } });
});
async function launch(command, args, env, label) {
  const log = await open(join(evidence, label + '.log'), 'w'); logs.push(log);
  const child = spawn(command, args, { env, detached: true, stdio: ['ignore', log.fd, log.fd] }); children.push(child);
  child.on('error', error => { child.failure = error; });
  return child;
}
async function freePort(port) {
  const server = createServer();
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
  await new Promise(resolve => server.close(resolve));
}
function value(result) {
  assert.equal(result.isError ?? false, false, JSON.stringify(result));
  return result.structuredContent ?? JSON.parse(result.content.find(block => block.type === 'text').text);
}
const http = httpServer((_request, response) => {
  response.setHeader('Content-Type', 'text/html');
  response.end(`<!doctype html><title>Dojo browser fixture</title><input aria-label="Name"><button onclick='document.querySelector("output").textContent=document.querySelector("input").value'>Apply</button><output>initial</output>`);
});
try {
  await freePort(19988); await freePort(19989);
  await new Promise(resolve => http.listen(0, '127.0.0.1', resolve));
  const original = await request(process.env.NIRI_SOCKET, 'Workspaces');
  const activeWorkspaces = workspaces => workspaces.filter(workspace => workspace.is_active).map(({ output, id }) => ({ output, id })).sort((a, b) => a.output.localeCompare(b.output));
  const initialActive = activeWorkspaces(original.Workspaces);
  const test = original.Workspaces.find(workspace => workspace.name === 'dojo-tests' && !workspace.is_active);
  assert(test, 'An inactive dojo-tests workspace is required');
  const nativeSocket = join(evidence, 'native.sock');
  const env = { ...process.env, DOJO_AGENT_INPUT: '1', DOJO_BROWSER_CONTROL_SOCKET: nativeSocket, DOJO_BROWSER_CONTROL_BROKER_EXE: broker, RUST_LOG: 'niri=info' };
  const niri = await launch(process.env.NIRI_BINARY ?? join(process.cwd(), 'vendor/niri/target/release/niri'), ['--config', join(process.cwd(), 'tools/agent-input/nested.kdl')], env, 'niri');
  const nested = await wait('nested compositor', async () => {
    if (niri.exitCode !== null || niri.failure) throw new Error('Nested compositor failed');
    const log = await readFile(join(evidence, 'niri.log'), 'utf8');
    const display = [...log.matchAll(/listening on Wayland socket: (\S+)/g)].at(-1)?.[1];
    const socket = [...log.matchAll(/IPC listening on: (\S+)/g)].at(-1)?.[1];
    return display && socket ? { display, socket } : undefined;
  });
  const hostWindow = await wait('nested host window', async () => (await request(process.env.NIRI_SOCKET, 'Windows')).Windows.find(window => window.pid === niri.pid));
  await request(process.env.NIRI_SOCKET, { Action: { MoveWindowToWorkspace: { window_id: hostWindow.id, reference: { Id: test.id }, focus: false } } });
  const outer = await request(process.env.NIRI_SOCKET, 'Workspaces');
  assert.deepEqual(activeWorkspaces(outer.Workspaces), initialActive);
  assert.equal(outer.Workspaces.find(workspace => workspace.id === test.id).is_active, false);
  const home = join(evidence, 'home'); await mkdir(home);
  const browserEnv = { ...env, HOME: home, WAYLAND_DISPLAY: nested.display, NIRI_SOCKET: nested.socket, BROWSER_CONTROL_DEBUG: 'true', BROWSER_CONTROL_AUTOSTART: 'false', DOJO_BROWSER_CONTROL_MODE: '1', DOJO_BROWSER_OWNER: 'dojo-live-test' };
  await launch(join(packagePath, 'bin/browser-control'), ['serve'], browserEnv, 'relay');
  await launch(process.env.DOJO_TEST_HELIUM ?? 'helium', ['--user-data-dir=' + join(evidence, 'profile'), '--ozone-platform=wayland', '--no-first-run', '--no-default-browser-check', '--load-extension=' + join(packagePath, 'lib/browser-control/extension/dist'), 'about:blank'], { ...env, WAYLAND_DISPLAY: nested.display }, 'helium');
  await wait('native browser window', async () => (await request(nested.socket, 'Windows')).Windows.find(window => /helium|chromium/i.test(window.app_id ?? '')), 30000);
  mcp = await DesktopMcp.open(process.cwd(), { command: join(packagePath, 'bin/browser-control-mcp'), args: [], env: browserEnv, timeoutMs: 90000, killGraceMs: 4000 });
  assert(mcp.tools.some(tool => tool.name === 'execute_cancel'));
  const call = async (name, args = {}) => mcp.call('tools/call', { name, arguments: args });
  await wait('extension readiness', async () => value(await call('status')).status?.connected === true);
  await new Promise(resolve => setTimeout(resolve, 250));
  const yielded = await request(nested.socket, 'YieldHumanInput');
  await writeFile(join(evidence, 'native-before-bind.json'), JSON.stringify({ yielded, windows: await request(nested.socket, 'Windows'), coordination: await request(nested.socket, 'InputCoordination') }, null, 2));
  const resumed = value(await call('session_resume'));
  const url = `http://127.0.0.1:${http.address().port}/`;
  const executed = value(await call('execute', { code: `await page.goto(${JSON.stringify(url)}); await page.getByRole('textbox', {name:'Name'}).fill('native path'); await page.getByRole('button', {name:'Apply'}).click(); return {title:await page.title(), value:await page.locator('output').textContent(), url:page.url()};` }));
  await writeFile(join(evidence, 'execute.json'), JSON.stringify(executed, null, 2));
  const returned = executed.value;
  assert.equal(returned?.value, 'native path', JSON.stringify(executed));
  const cancelled = value(await call('execute_cancel'));
  assert.equal(cancelled.nativeRetired, true);
  assert.deepEqual(cancelled.unknownCommandIds, []);
  const resumedAgain = value(await call('session_resume'));
  assert(resumedAgain.generation > resumed.generation);
  const preserved = value(await call('execute', { code: "return {url:page.url(), value:await page.locator('output').textContent()};" }));
  assert.equal(preserved.value?.value, 'native path', JSON.stringify(preserved));
  assert.equal(preserved.value?.url, url);
  const delayed = call('execute', { code: "await new Promise(resolve => setTimeout(resolve, 1000)); await page.locator('output').evaluate(element => element.textContent = 'late effect'); return page.url();" });
  await new Promise(resolve => setTimeout(resolve, 150));
  const interrupted = value(await call('execute_cancel'));
  assert.equal(interrupted.nativeRetired, true);
  const delayedResult = await delayed;
  assert.equal(delayedResult.isError, true, JSON.stringify(delayedResult));
  const afterInterrupt = value(await call('session_resume'));
  assert(afterInterrupt.generation > resumedAgain.generation);
  const noLateEffect = value(await call('execute', { code: "return await page.locator('output').textContent();" }));
  assert.equal(noLateEffect.value, 'native path', JSON.stringify(noLateEffect));
  const complete = value(await call('execute_cancel'));
  assert.equal(complete.nativeRetired, true);
  await writeFile(join(evidence, 'acceptance.json'), JSON.stringify({ initialActive, resumed, cancelled, resumedAgain, preserved, interrupted, delayedResult, afterInterrupt, noLateEffect, complete }, null, 2));
  const finalOuter = await request(process.env.NIRI_SOCKET, 'Workspaces');
  assert.deepEqual(activeWorkspaces(finalOuter.Workspaces), initialActive);
  assert.equal(finalOuter.Workspaces.find(workspace => workspace.id === test.id).is_active, false);
  await writeFile(join(evidence, 'outer-workspaces.json'), JSON.stringify({ initialActive, finalActive: activeWorkspaces(finalOuter.Workspaces), testWorkspace: test.id }, null, 2));
  console.log('PASS: real Helium fill/click/read through native writer; cancellation; fresh generation; preserved tab; active workspaces unchanged');
} finally {
  if (mcp) {
    const closed = new Promise(resolve => { if (mcp.exited) resolve(); else { mcp.child.once('exit', resolve); setTimeout(resolve, 4500).unref(); } });
    mcp.close(); await closed;
  }
  for (const child of children.reverse()) if (child.pid) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  await Promise.all(children.map(child => child.exitCode !== null ? undefined : new Promise(resolve => { child.once('exit', resolve); setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} resolve(); }, 4000).unref(); })));
  await new Promise(resolve => http.close(resolve));
  await Promise.all(logs.map(log => log.close()));
}
