#!/usr/bin/env node
// Runs all input and app mutations in a nested compositor on an inactive workspace.
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { connect } from 'node:net';
import { createReadStream } from 'node:fs';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, open, stat, realpath } from 'node:fs/promises';
import { basename, isAbsolute, join } from 'node:path';
import { tmpdir } from 'node:os';
import { createHash } from 'node:crypto';
import { DesktopMcp } from './desktop-extension.mjs';

if (process.argv.includes('--private-bus') && process.env.DOJO_TEST_PRIVATE_BUS !== '1') {
  assert(process.env.DBUS_SESSION_BUS_ADDRESS && process.env.XDG_RUNTIME_DIR && process.env.WAYLAND_DISPLAY, 'The original desktop session environment is required');
  const isolated = await mkdtemp(join(tmpdir(), 'djs.'));
  const runtime = join(isolated, 'r'), services = join(isolated, 'services');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(services);
  await copyFile(process.env.DOJO_TEST_A11Y_BUS_SERVICE ?? '/usr/share/dbus-1/services/org.a11y.Bus.service', join(services, 'org.a11y.Bus.service'));
  const config = join(isolated, 'session.conf');
  await writeFile(config, `<busconfig><type>session</type><listen>unix:tmpdir=${isolated}</listen><auth>EXTERNAL</auth><servicedir>${services}</servicedir><policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>`);
  const environment = { ...process.env, GSETTINGS_BACKEND: 'memory', XDG_RUNTIME_DIR: runtime, ATSPI_DBUS_IMPLEMENTATION: 'dbus-daemon',
    WAYLAND_DISPLAY: isAbsolute(process.env.WAYLAND_DISPLAY) ? process.env.WAYLAND_DISPLAY : join(process.env.XDG_RUNTIME_DIR, process.env.WAYLAND_DISPLAY),
    DOJO_TEST_PRIVATE_BUS: '1', DOJO_TEST_PRIVATE_ROOT: isolated, DOJO_TEST_ORIGINAL_SESSION_BUS: process.env.DBUS_SESSION_BUS_ADDRESS,
  };
  delete environment.AT_SPI_BUS_ADDRESS; delete environment.DBUS_STARTER_ADDRESS; delete environment.DBUS_STARTER_BUS_TYPE;
  const child = spawn('dbus-run-session', [`--config-file=${config}`, '--', process.execPath, process.argv[1]], { env: environment, stdio: 'inherit' });
  const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', (code) => resolve(code ?? 1)); });
  process.exit(code);
}

const root = process.cwd();
const gtk = process.env.DOJO_TEST_GTK_LIBRARY;
assert(gtk?.startsWith('/nix/store/') && gtk.endsWith('/lib/libgtk-4.so.1'), 'Set DOJO_TEST_GTK_LIBRARY to the immutable GTK4 library');
assert(process.env.NIRI_SOCKET, 'Run from the live Niri session inside devenv');
if (process.env.DOJO_TEST_ELECTRON_RUNTIME) {
  assert.equal(process.env.DOJO_TEST_PRIVATE_BUS, '1', 'Electron acceptance requires --private-bus');
  assert(process.env.DOJO_TEST_ELECTRON_BRIDGE_ADDON?.startsWith('/nix/store/') && process.env.DOJO_TEST_ELECTRON_BRIDGE_ADDON.endsWith('.node'), 'Supply the immutable native bridge fixture addon');
}
const niriBinary = process.env.NIRI_BINARY ?? join(root, 'vendor/niri/target/debug/niri');
const evidence = await mkdtemp(join(tmpdir(), 'dojo-semantic-live.'));
console.log(`Semantic acceptance evidence: ${evidence}`);
const children = [], logs = [], clients = [];
const results = { passed: false, evidence, niri_binary: niriBinary, gtk, checks: {} };
const delay = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
const hashFile = async path => {
  const hash = createHash('sha256');
  for await (const bytes of createReadStream(path)) hash.update(bytes);
  return hash.digest('hex');
};
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
const wait = async (label, check, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const value = await check(); if (value) return value; await delay(50); }
  throw new Error(`Timed out: ${label}`);
};
const active = workspaces => workspaces.filter(w => w.is_active).map(({ output, id }) => ({ output, id })).sort((a, b) => a.output.localeCompare(b.output));
const launch = async (command, args, env, label, input = false) => {
  const log = await open(join(evidence, `${label}.log`), 'w'); logs.push(log);
  const child = spawn(command, args, { env, detached: true, stdio: [input ? 'pipe' : 'ignore', log.fd, log.fd] });
  child.on('error', error => { child.failure = error; }); children.push(child); return child;
};
const value = result => {
  assert.equal(result.isError ?? false, false, JSON.stringify(result));
  return result.structuredContent ?? JSON.parse(result.content.find(block => block.type === 'text').text);
};
const call = (client, name, args) => client.call('tools/call', { name, arguments: args });
const observe = async (client, windowId) => {
  const tree = value(await call(client, 'desktop_accessibility', { window_id: windowId }));
  assert.equal(tree.scope, 'window', JSON.stringify(tree));
  assert.equal(tree.window_scoped, true);
  assert.equal(tree.available, true);
  assert.equal(tree.requested_window_id, windowId);
  assert(!JSON.stringify(tree).includes('fixture-sensitive-password'));
  return tree;
};
const node = (tree, name) => {
  const matching = tree.nodes.filter(n => n.name === name);
  const controls = matching.filter(n => !['label', 'static', 'static text'].includes(n.role));
  const found = controls.length ? controls : matching;
  assert.equal(found.length, 1, `Expected one ${name}: ${JSON.stringify(tree)}`);
  return found[0];
};
const observeWithRef = (client, windowId, name) => wait(`actionable ${name}`, async () => {
  const tree = await observe(client, windowId);
  const selected = node(tree, name);
  return typeof tree.element_refs?.[selected.index] === 'string' ? tree : undefined;
});
const ref = (tree, name) => {
  const token = tree.element_refs?.[node(tree, name).index];
  assert.equal(typeof token, 'string', `No actionable ref for ${name}: ${JSON.stringify(tree)}`);
  return token;
};
const refused = result => {
  const rejected = result.isError === true || value(result).effect === 'refused';
  assert(rejected, `Expected refusal: ${JSON.stringify(result)}`);
  return result;
};
const mutate = async (client, windowId, name, action) => {
  const before = await observeWithRef(client, windowId, name);
  const token = ref(before, name);
  results.last_attempt = { window_id: windowId, name, before,
    target_before: (await request(results.nested.socket, { AccessibilityTarget: { window_id: windowId } })).AccessibilityTarget,
    coordination_before: await request(results.nested.socket, 'InputCoordination') };
  const receipt = value(await call(client, 'desktop_accessibility_action', { element_ref: token, action }));
  results.last_attempt.receipt = receipt;
  results.last_attempt.target_after = (await request(results.nested.socket, { AccessibilityTarget: { window_id: windowId } })).AccessibilityTarget;
  results.last_attempt.coordination_after = await request(results.nested.socket, 'InputCoordination');
  assert.equal(receipt.effect, 'acknowledged', JSON.stringify(receipt));
  assert.equal(receipt.reference_consumed, true);
  assert.equal(receipt.verified, false);
  assert.equal(receipt.requires_fresh_observation, true);
  return { before, token, receipt };
};
let original;
try {
  if (process.env.DOJO_TEST_PRIVATE_BUS === '1') {
    assert(process.env.DOJO_TEST_ORIGINAL_SESSION_BUS, 'Record the original session bus before starting dbus-run-session');
    assert.notEqual(process.env.DBUS_SESSION_BUS_ADDRESS, process.env.DOJO_TEST_ORIGINAL_SESSION_BUS, 'Refusing to enable accessibility on the host session bus');
    assert.equal(process.env.GSETTINGS_BACKEND, 'memory', 'Private acceptance must use the memory settings backend');
    execFileSync('busctl', ['--address', process.env.DBUS_SESSION_BUS_ADDRESS, 'set-property', 'org.a11y.Bus', '/org/a11y/bus', 'org.a11y.Status', 'IsEnabled', 'b', 'true']);
    results.private_accessibility_bus = {
      session_bus: process.env.DBUS_SESSION_BUS_ADDRESS,
      isolated_from_host: true,
      runtime_directory: process.env.XDG_RUNTIME_DIR,
      private_root: process.env.DOJO_TEST_PRIVATE_ROOT,
      settings_backend: 'memory',
      status: execFileSync('busctl', ['--address', process.env.DBUS_SESSION_BUS_ADDRESS, 'get-property', 'org.a11y.Bus', '/org/a11y/bus', 'org.a11y.Status', 'IsEnabled'], { encoding: 'utf8' }).trim(),
    };
    assert.equal(results.private_accessibility_bus.status, 'b true');
  }
  let agent = process.env.DOJO_SEMANTIC_AGENT_EXE ?? join(root, 'target/debug/dojo-agent');
  if (!agent.startsWith('/nix/store/')) {
    assert.equal(basename(agent), 'dojo-agent', 'nix-store --add requires the built executable basename dojo-agent');
    agent = execFileSync('nix-store', ['--add', agent], { encoding: 'utf8' }).trim();
  }
  agent = await realpath(agent);
  const agentMetadata = await stat(agent);
  assert(agent.startsWith('/nix/store/') && agentMetadata.uid === 0 && (agentMetadata.mode & 0o222) === 0, 'Agent must be an immutable root-owned Nix-store executable');
  results.agent = agent;
  results.agent_sha256 = await hashFile(agent);
  results.niri_sha256 = await hashFile(niriBinary);
  original = (await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces;
  const focused = original.find(w => w.is_focused);
  const test = original.find(w => w.name === 'dojo-tests' && !w.is_active)
    ?? original.find(w => !w.is_active && !w.name && w.active_window_id === null && w.output === focused?.output);
  assert(test, 'An inactive dojo-tests or empty workspace is required');
  results.initial_active = active(original);
  results.test_workspace = test.id;
  const env = { ...process.env, DOJO_AGENT_INPUT: '1', DOJO_SEMANTIC_AGENT_EXE: agent, RUST_LOG: 'niri=info' };
  delete env.DOJO_BROWSER_CONTROL_BROKER_EXE;
  delete env.DOJO_BROWSER_CONTROL_SOCKET;
  delete env.AT_SPI_BUS_ADDRESS;
  delete env.DBUS_STARTER_ADDRESS;
  delete env.DBUS_STARTER_BUS_TYPE;
  const niri = await launch(niriBinary, ['--config', join(root, 'tools/agent-input/nested.kdl')], env, 'niri');
  const nested = await wait('nested compositor', async () => {
    if (niri.exitCode !== null || niri.failure) throw new Error('Nested compositor failed');
    const log = await readFile(join(evidence, 'niri.log'), 'utf8');
    const display = [...log.matchAll(/listening on Wayland socket: (\S+)/g)].at(-1)?.[1];
    const socket = [...log.matchAll(/IPC listening on: (\S+)/g)].at(-1)?.[1];
    return display && socket ? { display, socket } : undefined;
  });
  results.nested = nested;
  const host = await wait('nested host window', async () => (await request(process.env.NIRI_SOCKET, 'Windows')).Windows.find(w => w.pid === niri.pid));
  await request(process.env.NIRI_SOCKET, { Action: { MoveWindowToWorkspace: { window_id: host.id, reference: { Id: test.id }, focus: false } } });
  assert.deepEqual(active((await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces), active(original));
  const nestedEnv = { ...env, WAYLAND_DISPLAY: nested.display, NIRI_SOCKET: nested.socket, GTK_A11Y: 'atspi', GDK_BACKEND: 'wayland' };
  const humanPointer = process.env.DOJO_TEST_HUMAN_POINTER ?? join(root, 'tools/agent-input/target/debug/human-pointer');
  try { await stat(humanPointer); } catch {
    execFileSync('cargo', ['build', '--manifest-path', join(root, 'tools/agent-input/Cargo.toml'), '--bin', 'human-pointer'], { stdio: 'inherit' });
  }
  const outputs = (await request(nested.socket, 'Outputs')).Outputs;
  const logical = Object.values(outputs).find(output => output.logical)?.logical;
  assert(logical, 'Nested output logical dimensions unavailable');
  const pointer = (x, y) => execFileSync(humanPointer, [String(x), String(y), String(logical.width), String(logical.height)], { env: nestedEnv });
  pointer(logical.width - 2, logical.height - 2);
  const fixture = join(evidence, 'gtk-fixture');
  execFileSync('cc', [join(root, 'tools/agent-input/gtk-accessibility-fixture.c'), '-o', fixture, '-ldl']);
  const app = await launch(fixture, [gtk], nestedEnv, 'gtk');
  const windows = await wait('two same-process GTK windows', async () => {
    if (app.exitCode !== null || app.failure) throw new Error('GTK fixture failed');
    const found = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === app.pid);
    return found.length === 2 ? found : undefined;
  });
  assert.equal(windows[0].title, windows[1].title);
  const targets = await wait('two distinct exact GTK roots', async () => {
    const found = await Promise.all(windows.map(async w => (await request(nested.socket, { AccessibilityTarget: { window_id: w.id } })).AccessibilityTarget));
    return found.every(t => t.endpoint) ? found : undefined;
  });
  assert.notEqual(targets[0].endpoint.object_path, targets[1].endpoint.object_path);
  results.gtk_targets = targets;
  const mcp = await DesktopMcp.open(root, { command: agent, env: nestedEnv, timeoutMs: 15000 }); clients.push(mcp);
  assert(mcp.tools.some(tool => tool.name === 'desktop_accessibility_action'));
  await request(nested.socket, 'YieldHumanInput');
  const initial = await wait('two exclusive GTK control trees', async () => {
    const observations = [];
    for (const window of windows) observations.push({ window, tree: await observe(mcp, window.id) });
    results.initial_gtk_observations = observations;
    return observations.some(({ tree }) => tree.nodes.some(n => n.name === 'Alpha Save'))
      && observations.some(({ tree }) => tree.nodes.some(n => n.name === 'Beta Save')) ? observations : undefined;
  });
  const alpha = initial.find(({ tree }) => tree.nodes.some(n => n.name === 'Alpha Save'));
  const beta = initial.find(({ tree }) => tree.nodes.some(n => n.name === 'Beta Save'));
  assert(alpha && beta && alpha.window.id !== beta.window.id);
  assert(!alpha.tree.nodes.some(n => n.name === 'Beta Save'));
  assert(!beta.tree.nodes.some(n => n.name === 'Alpha Save'));
  results.checks.exact_sibling_roots = true;
  const originalRefTree = await observeWithRef(mcp, alpha.window.id, 'Alpha Input');
  const foreignRef = ref(originalRefTree, 'Alpha Input');
  const foreign = await DesktopMcp.open(root, { command: agent, env: nestedEnv }); clients.push(foreign);
  await observe(foreign, beta.window.id);
  results.foreign_ref = refused(await call(foreign, 'desktop_accessibility_action', { element_ref: foreignRef, action: { kind: 'replace_text', text: 'foreign must not run' } }));
  const foreignOwnRef = ref(await observeWithRef(foreign, alpha.window.id, 'Alpha Input'), 'Alpha Input');
  const input = await mutate(mcp, alpha.window.id, 'Alpha Input', { kind: 'replace_text', text: 'semantic replacement' });
  results.cross_session_stale_ref = refused(await call(foreign, 'desktop_accessibility_action', { element_ref: foreignOwnRef, action: { kind: 'replace_text', text: 'stale cross session must not run' } }));
  foreign.close();
  const afterInput = await wait('GTK text replacement', async () => {
    const tree = await observe(mcp, alpha.window.id);
    return tree.nodes.some(n => n.name === 'Alpha text: semantic replacement') && node(tree, 'Alpha Input').value === 'semantic replacement' ? tree : undefined;
  });
  const betaAfterInput = await observe(mcp, beta.window.id);
  assert.equal(node(betaAfterInput, 'Beta Input').value, 'initial');
  results.checks.text_replacement = { receipt: input.receipt, after: afterInput, sibling: betaAfterInput };
  results.replay = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: input.token, action: { kind: 'replace_text', text: 'replay must not run' } }));
  const numeric = await mutate(mcp, alpha.window.id, 'Alpha Value', { kind: 'set_numeric', value: 42 });
  const afterNumeric = await wait('GTK numeric value', async () => {
    const tree = await observe(mcp, alpha.window.id);
    return tree.nodes.some(n => n.name === 'Alpha value: 42') && Number(node(tree, 'Alpha Value').value) === 42 ? tree : undefined;
  });
  const betaAfterNumeric = await observe(mcp, beta.window.id);
  assert.equal(Number(node(betaAfterNumeric, 'Beta Value').value), 1);
  results.checks.numeric_value = { receipt: numeric.receipt, after: afterNumeric, sibling: betaAfterNumeric };
  const selection = await mutate(mcp, alpha.window.id, 'Alpha Choice Two', { kind: 'select' });
  const afterSelection = await wait('GTK direct row selection', async () => {
    const tree = await observe(mcp, alpha.window.id);
    return tree.nodes.some(n => n.name === 'Alpha selection: 2') && node(tree, 'Alpha Choice Two').selected === true ? tree : undefined;
  });
  const betaAfterSelection = await observe(mcp, beta.window.id);
  assert.equal(node(betaAfterSelection, 'Beta Choice One').selected, true);
  assert.equal(node(betaAfterSelection, 'Beta Choice Two').selected, false);
  assert(betaAfterSelection.nodes.some(n => n.name === 'Beta selection: 1'));
  results.checks.direct_row_selection = { receipt: selection.receipt, after: afterSelection, sibling: betaAfterSelection };
  const invoked = await mutate(mcp, alpha.window.id, 'Alpha Save', { kind: 'invoke' });
  const afterInvoke = await wait('GTK invoked button changes application state', async () => {
    const tree = await observe(mcp, alpha.window.id);
    return tree.nodes.some(n => n.name === 'Saved') && node(tree, 'Alpha Save').enabled === false ? tree : undefined;
  });
  const betaAfterInvoke = await observe(mcp, beta.window.id);
  assert.equal(node(betaAfterInvoke, 'Beta Save').enabled, true);
  assert(betaAfterInvoke.nodes.some(n => n.name === 'Beta pending'));
  assert(!betaAfterInvoke.nodes.some(n => n.name === 'Saved'));
  results.checks.invoke = { receipt: invoked.receipt, after: afterInvoke, sibling: betaAfterInvoke };
  const staleTree = await observeWithRef(mcp, alpha.window.id, 'Alpha Input');
  const stale = ref(staleTree, 'Alpha Input');
  await observe(mcp, beta.window.id);
  results.stale_snapshot = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: stale, action: { kind: 'replace_text', text: 'stale must not run' } }));
  const expiringRef = ref(await observeWithRef(mcp, alpha.window.id, 'Alpha Input'), 'Alpha Input');
  await delay(15100);
  results.expired_ref = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: expiringRef, action: { kind: 'replace_text', text: 'expired must not run' } }));
  assert.match(JSON.stringify(results.expired_ref), /expired/);

  // Virtual primary-seat motion enters this nested client and is handled by the
  // same native human activity path as real input. It never touches the host seat.
  await request(nested.socket, { Action: { MoveWindowToFloating: { id: alpha.window.id } } });
  await request(nested.socket, { Action: { MoveFloatingWindow: { id: alpha.window.id, x: { SetFixed: 40 }, y: { SetFixed: 40 } } } });
  await request(nested.socket, { Action: { FocusWindow: { id: alpha.window.id } } });
  pointer(logical.width - 2, logical.height - 2);
  await request(nested.socket, 'YieldHumanInput');
  const takeoverTree = await observeWithRef(mcp, alpha.window.id, 'Alpha Input');
  const takeoverRef = ref(takeoverTree, 'Alpha Input');
  const beforeTakeover = (await request(nested.socket, { AccessibilityTarget: { window_id: alpha.window.id } })).AccessibilityTarget;
  pointer(80, 80);
  const afterTakeover = await wait('nested human takeover', async () => {
    const target = (await request(nested.socket, { AccessibilityTarget: { window_id: alpha.window.id } })).AccessibilityTarget;
    return target.input_generation > beforeTakeover.input_generation ? target : undefined;
  });
  results.human_takeover = { before: beforeTakeover, after: afterTakeover,
    refused: refused(await call(mcp, 'desktop_accessibility_action', { element_ref: takeoverRef, action: { kind: 'replace_text', text: 'human takeover must prevent this' } })) };
  pointer(logical.width - 2, logical.height - 2);
  await request(nested.socket, 'YieldHumanInput');
  const unchanged = await observe(mcp, alpha.window.id);
  assert.equal(node(unchanged, 'Alpha Input').value, 'semantic replacement');
  assert(unchanged.nodes.some(n => n.name === 'Alpha text: semantic replacement'));
  results.human_takeover.after_observation = unchanged;
  results.checks.human_takeover_no_effect = true;

  results.runtime_matrix = [
    { runtime: 'GTK 4.22.4 fixture', binding: 'exact GTK7 root', invoke: true, replace_text: true, set_numeric: true, select: true, sibling_isolation: true },
  ];
  for (const [major, version, qtPackage] of [
    [6, '6.11.2', process.env.DOJO_TEST_QT6_ANNOTATION_PACKAGE ?? '/nix/store/5gk3mlfs2jswgngkvbf31fycbq1qfd7g-dojo-qt6-annotation-6.11.2'],
    [5, '5.15.19', process.env.DOJO_TEST_QT5_ANNOTATION_PACKAGE ?? '/nix/store/pkazn6a91rwcsvg1751paxvlchxx8rf2-dojo-qt5-annotation-5.15.19'],
  ]) {
    assert(qtPackage.startsWith('/nix/store/'), 'Qt fixture package must be immutable');
    const qtEnv = { ...nestedEnv, QT_QPA_PLATFORM: 'wayland', QT_LINUX_ACCESSIBILITY_ALWAYS_ON: '1',
      QT_PLUGIN_PATH: `${qtPackage}/lib/dojo/qt-${version}/plugins${nestedEnv.QT_PLUGIN_PATH ? ':' + nestedEnv.QT_PLUGIN_PATH : ''}`,
      QT_QPA_GENERIC_PLUGINS: `dojo-qt${major}-annotation-${version}${nestedEnv.QT_QPA_GENERIC_PLUGINS ? ',' + nestedEnv.QT_QPA_GENERIC_PLUGINS : ''}` };
    if (major === 5) qtEnv.QT_QPA_PLATFORM_PLUGIN_PATH = `${qtPackage}/lib/dojo/qt-${version}/qpa-gate`;
    delete qtEnv.LD_LIBRARY_PATH;
    const qt = await launch(join(qtPackage, `libexec/dojo_qt${major}_annotation_fixture`), [], qtEnv, `qt${major}`);
    const qtWindows = await wait(`two Qt${major} native windows`, async () => {
      if (qt.exitCode !== null || qt.failure) throw new Error(`Qt${major} fixture failed`);
      const found = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === qt.pid);
      return found.length === 2 ? found : undefined;
    });
    const qtTargets = await wait(`two generic annotated Qt${major} roots`, async () => {
      const found = await Promise.all(qtWindows.map(async w => (await request(nested.socket, { AccessibilityTarget: { window_id: w.id } })).AccessibilityTarget));
      return found.every(t => t.endpoint && t.annotation_generation > 0) ? found : undefined;
    });
    assert.notEqual(qtTargets[0].endpoint.object_path, qtTargets[1].endpoint.object_path);
    await request(nested.socket, 'YieldHumanInput');
    const replacement = `Qt${major} exact semantic text`;
    const qtMutation = await mutate(mcp, qtWindows[0].id, 'Fixture input', { kind: 'replace_text', text: replacement });
    const qtAfter = await wait(`Qt${major} exact text replacement`, async () => {
      const tree = await observe(mcp, qtWindows[0].id);
      return node(tree, 'Fixture input').value === replacement ? tree : undefined;
    });
    const qtSibling = await observe(mcp, qtWindows[1].id);
    assert.notEqual(node(qtSibling, 'Fixture input').value, replacement);
    results.checks[`qt${major}_generic_roots_and_text`] = { package: qtPackage, targets: qtTargets, receipt: qtMutation.receipt, after: qtAfter, sibling: qtSibling };
    results.runtime_matrix.push({ runtime: `Qt ${version} fixture`, package: qtPackage, binding: 'exact generic root', replace_text: true, sibling_isolation: true });
  }

  const gpuiBinary = process.env.DOJO_TEST_GPUI_FIXTURE ?? join(root, 'target/debug/dojo-accessibility-fixture');
  const gpui = await launch(gpuiBinary, [], nestedEnv, 'gpui');
  const gpuiWindows = await wait('two GPUI native windows', async () => {
    if (gpui.exitCode !== null || gpui.failure) throw new Error('GPUI fixture failed');
    const found = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === gpui.pid);
    return found.length === 2 ? found : undefined;
  });
  results.gpui_windows = gpuiWindows;
  const gpuiTargets = await wait('two generic annotated GPUI roots', async () => {
    const found = await Promise.all(gpuiWindows.map(async w => (await request(nested.socket, { AccessibilityTarget: { window_id: w.id } })).AccessibilityTarget));
    results.gpui_targets = found;
    return found.every(t => t.endpoint && t.annotation_generation > 0) ? found : undefined;
  });
  assert.notEqual(gpuiTargets[0].endpoint.object_path, gpuiTargets[1].endpoint.object_path);
  await request(nested.socket, 'YieldHumanInput');
  const gpuiObserved = [];
  for (const window of gpuiWindows) gpuiObserved.push({ window, tree: await observe(mcp, window.id) });
  const gpuiAlphaObservation = gpuiObserved.find(({ tree }) => tree.nodes.some(n => n.name === 'Alpha action'));
  const gpuiBetaObservation = gpuiObserved.find(({ tree }) => tree.nodes.some(n => n.name === 'Beta action'));
  assert(gpuiAlphaObservation && gpuiBetaObservation);
  const gpuiAlpha = gpuiAlphaObservation.window, gpuiBeta = gpuiBetaObservation.window;
  const gpuiBeforeAlpha = gpuiAlphaObservation.tree, gpuiBeforeBeta = gpuiBetaObservation.tree;
  node(gpuiBeforeAlpha, 'Alpha action'); node(gpuiBeforeBeta, 'Beta action');
  assert(!gpuiBeforeAlpha.nodes.some(n => n.name === 'Beta action'));
  assert(!gpuiBeforeBeta.nodes.some(n => n.name === 'Alpha action'));
  const gpuiMutation = await mutate(mcp, gpuiAlpha.id, 'Alpha action', { kind: 'invoke' });
  const invokedText = tree => tree.nodes.some(n => [n.name, n.text, n.value].includes('Alpha action invoked'));
  const gpuiAfter = await wait('GPUI invoked text', async () => {
    const tree = await observe(mcp, gpuiAlpha.id);
    return invokedText(tree) ? tree : undefined;
  });
  const gpuiSibling = await observe(mcp, gpuiBeta.id);
  assert(!gpuiSibling.nodes.some(n => [n.name, n.text, n.value].includes('Beta action invoked')));
  assert(!invokedText(gpuiSibling));
  results.checks.gpui_generic_roots_and_invoke = { binary: gpuiBinary, targets: gpuiTargets, receipt: gpuiMutation.receipt, after: gpuiAfter, sibling: gpuiSibling };
  results.runtime_matrix.push({ runtime: 'repository GPUI fixture', binary: gpuiBinary, binding: 'exact generic root', invoke: true, sibling_isolation: true });
  if (process.env.DOJO_TEST_ELECTRON_RUNTIME) {
    const runtime = process.env.DOJO_TEST_ELECTRON_RUNTIME;
    assert(isAbsolute(runtime), 'Use an absolute packaged Electron runtime');
    const electronEnv = { ...nestedEnv, NO_AT_BRIDGE: '0', DOJO_ELECTRON_FIXTURE_PROFILE: join(evidence, 'electron-profile'),
      DOJO_ELECTRON_FIXTURE_BRIDGE_ADDON: process.env.DOJO_TEST_ELECTRON_BRIDGE_ADDON };
    delete electronEnv.LD_LIBRARY_PATH;
    delete electronEnv.LD_PRELOAD;
    delete electronEnv.ELECTRON_RUN_AS_NODE;
    const versions = JSON.parse(execFileSync(runtime, ['-p', 'JSON.stringify(process.versions)'], {
      env: { ...electronEnv, ELECTRON_RUN_AS_NODE: '1' }, encoding: 'utf8', timeout: 15000,
    }));
    const expected = JSON.parse(await readFile(join(root, 'provenance/native-electron-investigation.json'), 'utf8'));
    assert.equal(versions.electron, expected.electron.version);
    assert.equal(versions.chrome, expected.chromium.version);
    const electron = await launch(runtime, ['--ozone-platform=wayland', join(root, 'tools/accessibility/electron/fixture.cjs')], electronEnv, 'electron', true);
    electron.stdin.on('error', error => { electron.failure = error; });
    const mapped = () => wait('two Electron native windows', async () => {
      if (electron.exitCode !== null || electron.failure) throw new Error('Electron fixture failed');
      const found = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === electron.pid);
      return found.length === 2 ? found : undefined;
    });
    const exact = async window => (await request(nested.socket, { AccessibilityTarget: { window_id: window.id } })).AccessibilityTarget;
    const electronWindows = await mapped();
    assert.equal(electronWindows[0].title, electronWindows[1].title);
    const electronTargets = await wait('distinct generic Electron roots', async () => {
      const targets = await Promise.all(electronWindows.map(exact));
      return targets.every(target => target.endpoint && target.annotation_generation > 0) ? targets : undefined;
    });
    assert.notEqual(electronTargets[0].endpoint.object_path, electronTargets[1].endpoint.object_path);
    await request(nested.socket, 'YieldHumanInput');
    const observations = await wait('separate Electron controls', async () => {
      const found = [];
      for (const window of electronWindows) found.push({ window, tree: await observe(mcp, window.id) });
      return found.some(({ tree }) => tree.nodes.some(n => n.name === 'Alpha electron input'))
        && found.some(({ tree }) => tree.nodes.some(n => n.name === 'Beta electron input')) ? found : undefined;
    });
    const first = observations.find(({ tree }) => tree.nodes.some(n => n.name === 'Alpha electron input'));
    const sibling = observations.find(({ tree }) => tree.nodes.some(n => n.name === 'Beta electron input'));
    assert(!first.tree.nodes.some(n => n.name === 'Beta electron input'));
    assert(!sibling.tree.nodes.some(n => n.name === 'Alpha electron input'));
    const text = await mutate(mcp, first.window.id, 'Alpha electron input', { kind: 'replace_text', text: 'Electron exact semantic text' });
    const afterText = await wait('Electron text replacement', async () => {
      const tree = await observe(mcp, first.window.id);
      return node(tree, 'Alpha electron input').value === 'Electron exact semantic text' ? tree : undefined;
    });
    const numeric = await mutate(mcp, first.window.id, 'Alpha electron value', { kind: 'set_numeric', value: 42 });
    const afterNumeric = await wait('Electron numeric value', async () => {
      const tree = await observe(mcp, first.window.id);
      return Number(node(tree, 'Alpha electron value').value) === 42 ? tree : undefined;
    });
    const selection = await mutate(mcp, first.window.id, 'Alpha electron choice two', { kind: 'select' });
    const afterSelection = await wait('Electron selection', async () => {
      const tree = await observe(mcp, first.window.id);
      return node(tree, 'Alpha electron choice two').selected === true ? tree : undefined;
    });
    const invoked = await mutate(mcp, first.window.id, 'Alpha electron action', { kind: 'invoke' });
    const afterInvoke = await wait('Electron invoked action', async () => {
      const tree = await observe(mcp, first.window.id);
      return tree.nodes.some(n => n.name === 'Alpha electron action invoked') ? tree : undefined;
    });
    const siblingAfter = await observe(mcp, sibling.window.id);
    assert.equal(node(siblingAfter, 'Beta electron input').value, 'initial');
    assert.equal(Number(node(siblingAfter, 'Beta electron value').value), 1);
    assert.equal(node(siblingAfter, 'Beta electron choice one').selected, true);
    assert.equal(node(siblingAfter, 'Beta electron choice two').selected, false);
    assert(!siblingAfter.nodes.some(n => n.name === 'Alpha electron action invoked'));
    const disabledRef = ref(await observeWithRef(mcp, first.window.id, 'Alpha electron input'), 'Alpha electron input');
    electron.stdin.write(JSON.stringify({ command: 'disable-accessibility', window: 'Alpha' }) + '\n');
    await wait('Electron accessibility disable withdraws both roots', async () => {
      const targets = await Promise.all(electronWindows.map(exact));
      return targets.every(target => !target.endpoint);
    });
    const disabledRefused = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: disabledRef,
      action: { kind: 'replace_text', text: 'disabled accessibility must not run' } }));
    electron.stdin.write(JSON.stringify({ command: 'enable-accessibility', window: 'Alpha' }) + '\n');
    const enabledTargets = await wait('Electron accessibility re-enable publishes both roots', async () => {
      const targets = await Promise.all(electronWindows.map(exact));
      return targets.every(target => target.endpoint && target.annotation_generation > 0) ? targets : undefined;
    });
    assert.notEqual(enabledTargets[0].endpoint.object_path, enabledTargets[1].endpoint.object_path);
    const enabledTree = await wait('Electron controls survive accessibility disable', async () => {
      const tree = await observe(mcp, first.window.id);
      return tree.nodes.some(n => n.name === 'Alpha electron input')
        && node(tree, 'Alpha electron input').value === 'Electron exact semantic text' ? tree : undefined;
    });
    const beforeBridge = await exact(first.window);
    const bridgeRef = ref(await observeWithRef(mcp, first.window.id, 'Alpha electron input'), 'Alpha electron input');
    electron.stdin.write(JSON.stringify({ command: 'cleanup-bridge', window: 'Alpha' }) + '\n');
    await wait('Electron bridge cleanup withdraws both roots', async () => {
      const targets = await Promise.all(electronWindows.map(exact));
      return targets.every(target => !target.endpoint);
    });
    const bridgeRefused = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: bridgeRef,
      action: { kind: 'replace_text', text: 'retired bridge must not run' } }));
    electron.stdin.write(JSON.stringify({ command: 'initialize-bridge', window: 'Alpha' }) + '\n');
    const afterBridge = await wait('Electron bridge reinitialization publishes both roots', async () => {
      const targets = await Promise.all(electronWindows.map(exact));
      return targets.every(target => target.endpoint && target.annotation_generation > 0) ? targets : undefined;
    });
    assert.notEqual(afterBridge[0].endpoint.bus_name, beforeBridge.endpoint.bus_name);
    assert.notEqual(afterBridge[0].endpoint.object_path, afterBridge[1].endpoint.object_path);
    const bridgeRestoredTree = await wait('Electron live controls survive bridge reinitialization', async () => {
      const tree = await observe(mcp, first.window.id);
      return tree.nodes.some(n => n.name === 'Alpha electron input')
        && node(tree, 'Alpha electron input').value === 'Electron exact semantic text' ? tree : undefined;
    });
    const hiddenRef = ref(await observeWithRef(mcp, first.window.id, 'Alpha electron input'), 'Alpha electron input');
    electron.stdin.write(JSON.stringify({ command: 'hide', window: 'Alpha' }) + '\n');
    await wait('Electron hidden surface retirement', async () => !(await request(nested.socket, 'Windows')).Windows.some(w => w.id === first.window.id));
    const hiddenRefused = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: hiddenRef, action: { kind: 'replace_text', text: 'hidden target must not run' } }));
    electron.stdin.write(JSON.stringify({ command: 'show', window: 'Alpha' }) + '\n');
    const shownWindows = await mapped();
    await request(nested.socket, 'YieldHumanInput');
    const shown = await wait('Electron shown exact root', async () => {
      for (const window of shownWindows) {
        if (!(await exact(window)).endpoint) continue;
        const tree = await observe(mcp, window.id);
        if (tree.nodes.some(n => n.name === 'Alpha electron input')) return { window, tree, target: await exact(window) };
      }
    });
    assert.equal(node(shown.tree, 'Alpha electron input').value, 'Electron exact semantic text');
    const recreatedRef = ref(await observeWithRef(mcp, shown.window.id, 'Alpha electron input'), 'Alpha electron input');
    electron.stdin.write(JSON.stringify({ command: 'recreate', window: 'Alpha' }) + '\n');
    await wait('Electron old native window retirement', async () => !(await request(nested.socket, 'Windows')).Windows.some(w => w.id === shown.window.id));
    const recreatedRefused = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: recreatedRef, action: { kind: 'replace_text', text: 'retired target must not run' } }));
    const recreatedWindows = await mapped();
    const recreated = await wait('Electron replacement root', async () => {
      for (const window of recreatedWindows) {
        const target = await exact(window);
        if (!target.endpoint) continue;
        const tree = await observe(mcp, window.id);
        if (tree.nodes.some(n => n.name === 'Alpha electron input')) return { window, tree, target };
      }
    });
    assert.notEqual(recreated.target.endpoint.object_path, shown.target.endpoint.object_path);
    assert.equal(node(recreated.tree, 'Alpha electron input').value, 'initial');
    results.checks.electron_generic_roots_actions_and_lifecycle = { runtime, versions, targets: electronTargets,
      text: text.receipt, numeric: numeric.receipt, selection: selection.receipt, invoke: invoked.receipt, afterText, afterNumeric, afterSelection, afterInvoke, siblingAfter, disabledRefused, enabledTargets, enabledTree, beforeBridge, afterBridge, bridgeRefused, bridgeRestoredTree, shown, recreated, hiddenRefused, recreatedRefused };
    results.runtime_matrix.push({ runtime: `Electron ${versions.electron} / Chromium ${versions.chrome} fixture`, binary: runtime,
      binary_sha256: await hashFile(runtime), binding: 'exact generic root', replace_text: true, set_numeric: true, select: true, invoke: true,
      sibling_isolation: true, accessibility_disable_reenable: true, bridge_cleanup_reinitialization: true, hide_show: true, root_recreation: true });
  }
  results.coverage_limit = 'Only the fixture runtimes listed in runtime_matrix were exercised. Other builds, bundled runtimes and unbound applications are not established by this acceptance run.';

  const closingTree = await observeWithRef(mcp, alpha.window.id, 'Alpha Input');
  const closingRef = ref(closingTree, 'Alpha Input');
  app.kill('SIGTERM');
  await wait('GTK target retirement', async () => !(await request(nested.socket, 'Windows')).Windows.some(w => w.pid === app.pid));
  results.closed_target = refused(await call(mcp, 'desktop_accessibility_action', { element_ref: closingRef, action: { kind: 'replace_text', text: 'closed must not run' } }));
  assert.equal((await call(mcp, 'desktop_accessibility', { window_id: alpha.window.id })).isError, true);
  results.checks.closed_target_refused = true;
  const final = (await request(process.env.NIRI_SOCKET, 'Workspaces')).Workspaces;
  assert.deepEqual(active(final), active(original));
  assert.equal(final.find(w => w.id === test.id).is_active, false);
  results.final_active = active(final);
  results.passed = true;
  console.log(`PASS: semantic actions, exact sibling roots, reference/session/expiry checks, human takeover, and retirement across ${results.runtime_matrix.length} listed fixture runtimes.`);
} catch (error) {
  results.error = error.stack ?? String(error);
  throw error;
} finally {
  await writeFile(join(evidence, 'acceptance.json'), JSON.stringify(results, null, 2));
  for (const client of clients) client.close();
  for (const child of children.reverse()) if (child.pid) { try { process.kill(-child.pid, 'SIGTERM'); } catch {} }
  await Promise.all(children.map(child => child.exitCode !== null || child.signalCode !== null ? undefined : new Promise(resolve => {
    const timeout = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} resolve(); }, 2000);
    child.once('exit', () => { clearTimeout(timeout); resolve(); });
  })));
  await Promise.all(logs.map(log => log.close()));
}
