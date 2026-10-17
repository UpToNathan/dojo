#!/usr/bin/env node
// Runs corpus tasks against fixtures in a nested compositor on an inactive
// workspace, then scores each run with the runner's own desktop_verify calls.
// Isolation steps mirror check-semantic-accessibility-live.mjs.
//
//   --agent scripted   deterministic semantic baseline: fixed tool calls, no model, no spend
//   --agent dojo       the real Dojo agent: the same PiRuntime (pi-durable) and dojo-agent MCP the voice
//                      service uses, on a fresh in-memory conversation with the live service's saved model account
//   --only a,b         run listed task ids
//   --list             print runnable and skipped tasks without starting anything
//   --provider/--model optional dojo-agent override; the default is the live service's saved model selection
//   --task-timeout-ms  dojo-agent per-task deadline, default 180000
//   --agent-retries    retries after a provider error before reporting agent_error, default 1
//
// Defaults: the installed Niri build (~/.local/share/dojo/niri/current/niri), the trusted
// agent and perception worker named by that installation's environment.sh, and the first immutable GTK4 library
// that actually loads under this shell's compiler. Override with NIRI_BINARY,
// DOJO_SEMANTIC_AGENT_EXE and DOJO_TEST_GTK_LIBRARY. Results land in
// <evidence>/results.json; the stdout summary omits recorded events.
import assert from 'node:assert/strict';
import { spawn, spawnSync, execFileSync } from 'node:child_process';
import { connect } from 'node:net';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, open, stat, realpath, symlink, readdir, rm } from 'node:fs/promises';
import { basename, isAbsolute, join } from 'node:path';
import { tmpdir, homedir } from 'node:os';
import { readdirSync } from 'node:fs';
import { DesktopMcp } from './desktop-extension.mjs';
import { validateCorpus, scoreRun, skipped, agentError, summarize } from './task-corpus.mjs';

const args = process.argv.slice(2);
const option = name => { const index = args.indexOf(name); return index < 0 ? undefined : args[index + 1]; };
const agentKind = option('--agent') ?? 'scripted';
assert(['scripted', 'dojo'].includes(agentKind), '--agent must be scripted or dojo');
const only = option('--only')?.split(',');
const root = process.cwd();
const corpus = JSON.parse(await readFile(join(root, 'tools/speech/task-corpus.json'), 'utf8'));
const invalid = validateCorpus(corpus);
assert.deepEqual(invalid, [], 'Corpus is invalid');
const unknownIds = (only ?? []).filter(id => !corpus.tasks.some(task => task.id === id));
assert.deepEqual(unknownIds, [], `Unknown task ids: ${unknownIds}`);
const selected = corpus.tasks.filter(task => !only || only.includes(task.id));
const RUNNABLE = new Set(['gtk', 'electron-seat']);
if (args.includes('--list')) {
  for (const task of selected) console.log(`${RUNNABLE.has(task.fixture) ? 'run ' : 'skip'} ${task.id} (${task.tier}, fixture ${task.fixture})`);
  process.exit(0);
}
const liveDir = join(process.env.XDG_STATE_HOME ?? join(homedir(), '.local/state'), 'dojo/voice');
const explicitModel = option('--provider') && option('--model') ? { provider: option('--provider'), model: option('--model') } : undefined;
let liveSelection;
if (agentKind === 'dojo') {
  liveSelection = JSON.parse(await readFile(join(liveDir, 'config.json'), 'utf8').catch(() => '{}')).modelSelection;
  assert(explicitModel || liveSelection?.account, 'No saved model selection in the live voice config; pass --provider and --model');
}

if (process.env.DOJO_TEST_PRIVATE_BUS !== '1') {
  assert(process.env.DBUS_SESSION_BUS_ADDRESS && process.env.XDG_RUNTIME_DIR && process.env.WAYLAND_DISPLAY, 'The original desktop session environment is required');
  const isolated = await mkdtemp(join(tmpdir(), 'djc.'));
  const runtime = join(isolated, 'r'), services = join(isolated, 'services');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(services);
  await copyFile(process.env.DOJO_TEST_A11Y_BUS_SERVICE ?? '/usr/share/dbus-1/services/org.a11y.Bus.service', join(services, 'org.a11y.Bus.service'));
  const config = join(isolated, 'session.conf');
  await writeFile(config, `<busconfig><type>session</type><listen>unix:tmpdir=${isolated}</listen><auth>EXTERNAL</auth><servicedir>${services}</servicedir><policy context="default"><allow own="*"/><allow send_destination="*"/><allow receive_sender="*"/></policy></busconfig>`);
  const environment = { ...process.env, GSETTINGS_BACKEND: 'memory', XDG_RUNTIME_DIR: runtime, ATSPI_DBUS_IMPLEMENTATION: 'dbus-daemon',
    WAYLAND_DISPLAY: isAbsolute(process.env.WAYLAND_DISPLAY) ? process.env.WAYLAND_DISPLAY : join(process.env.XDG_RUNTIME_DIR, process.env.WAYLAND_DISPLAY),
    DOJO_TEST_PRIVATE_BUS: '1', DOJO_TEST_PRIVATE_ROOT: isolated, DOJO_TEST_ORIGINAL_SESSION_BUS: process.env.DBUS_SESSION_BUS_ADDRESS };
  delete environment.AT_SPI_BUS_ADDRESS; delete environment.DBUS_STARTER_ADDRESS; delete environment.DBUS_STARTER_BUS_TYPE;
  if (agentKind === 'dojo') {
    // The account key lives in the Secret Service on the host session bus, which the private accessibility bus
    // cannot reach. Read it once here, read-only, and pass it through a 0600 file in this private directory.
    const account = explicitModel?.provider ?? liveSelection.account;
    const lookup = spawnSync('secret-tool', ['lookup', 'service', 'dojo', 'account', account], { encoding: 'utf8' });
    let credential = lookup.status === 0 && lookup.stdout.trim() ? JSON.parse(lookup.stdout) : undefined;
    credential ??= JSON.parse(await readFile(join(liveDir, 'pi/credentials.json'), 'utf8').catch(() => '{}'))[account];
    assert(credential, `No stored ${account} credential in the live Dojo store`);
    const credentialsFile = join(isolated, 'credentials.json');
    await writeFile(credentialsFile, JSON.stringify({ [account]: credential }), { mode: 0o600 });
    environment.DOJO_TEST_CREDENTIALS_FILE = credentialsFile;
  }
  const child = spawn('dbus-run-session', [`--config-file=${config}`, '--', process.execPath, process.argv[1], ...args], { env: environment, stdio: 'inherit' });
  const code = await new Promise((resolve, reject) => { child.once('error', reject); child.once('exit', code => resolve(code ?? 1)); });
  await rm(isolated, { recursive: true, force: true });
  process.exit(code);
}

const installation = join(process.env.XDG_DATA_HOME ?? join(homedir(), '.local/share'), 'dojo/niri/current');
let gtk = process.env.DOJO_TEST_GTK_LIBRARY;
if (gtk) assert(gtk.startsWith('/nix/store/') && gtk.endsWith('/lib/libgtk-4.so.1'), 'DOJO_TEST_GTK_LIBRARY must be an immutable GTK4 library');
assert(process.env.NIRI_SOCKET, 'Run from the live Niri session inside devenv');
assert.notEqual(process.env.DBUS_SESSION_BUS_ADDRESS, process.env.DOJO_TEST_ORIGINAL_SESSION_BUS, 'Refusing to enable accessibility on the host session bus');
assert.equal(process.env.GSETTINGS_BACKEND, 'memory');
const hostSocket = process.env.NIRI_SOCKET;
const niriBinary = process.env.NIRI_BINARY ?? join(installation, 'niri');
await stat(niriBinary).catch(() => assert.fail(`Niri build not found: ${niriBinary}`));
const evidence = await mkdtemp(join(tmpdir(), 'dojo-task-corpus.'));
console.log(`Task corpus evidence: ${evidence}`);
const children = [], logs = [], clients = [];
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
const request = (path, value) => new Promise((resolve, reject) => {
  const socket = connect(path); let text = '';
  socket.setTimeout(3000, () => socket.destroy(new Error('Niri IPC deadline')));
  socket.on('error', reject);
  socket.on('connect', () => socket.write(JSON.stringify(value) + '\n'));
  socket.on('data', bytes => {
    text += bytes; if (!text.includes('\n')) return;
    socket.end();
    try { const result = JSON.parse(text.split('\n')[0]); if (result.Err) reject(new Error(result.Err)); else resolve(result.Ok); } catch (error) { reject(error); }
  });
});
const wait = async (label, check, timeout = 20000) => {
  const end = Date.now() + timeout;
  while (Date.now() < end) { const value = await check(); if (value) return value; await delay(50); }
  throw new Error(`Timed out: ${label}`);
};
const launch = async (command, argv, env, label) => {
  const log = await open(join(evidence, `${label}.log`), 'w'); logs.push(log);
  const child = spawn(command, argv, { env, detached: true, stdio: ['ignore', log.fd, log.fd] });
  child.on('error', error => { child.failure = error; }); children.push(child); return child;
};
const stop = async child => {
  if (!child.pid || child.exitCode !== null || child.signalCode !== null) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  await new Promise(resolve => {
    const timeout = setTimeout(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} resolve(); }, 2000);
    child.once('exit', () => { clearTimeout(timeout); resolve(); });
  });
};
const value = result => {
  assert.equal(result.isError ?? false, false, JSON.stringify(result));
  return result.structuredContent ?? JSON.parse(result.content.find(block => block.type === 'text').text);
};
const call = (client, name, input) => client.call('tools/call', { name, arguments: input });
const active = workspaces => workspaces.filter(w => w.is_active).map(({ output, id }) => ({ output, id })).sort((a, b) => a.output.localeCompare(b.output));
const results = { agent: agentKind, evidence, runs: [] };
let original;
try {
  const installedEnvironment = await readFile(join(installation, 'environment.sh'), 'utf8').catch(() => '');
  const installed = installedEnvironment.match(/^export DOJO_SEMANTIC_AGENT_EXE=(\S+)$/m)?.[1];
  const perception = process.env.DOJO_PERCEPTION_INSTALLATION ?? installedEnvironment.match(/^export DOJO_PERCEPTION_INSTALLATION=(\S+)$/m)?.[1];
  let agent = process.env.DOJO_SEMANTIC_AGENT_EXE ?? installed ?? join(root, 'target/debug/dojo-agent');
  if (!agent.startsWith('/nix/store/')) {
    assert.equal(basename(agent), 'dojo-agent');
    agent = execFileSync('nix-store', ['--add', agent], { encoding: 'utf8' }).trim();
  }
  agent = await realpath(agent);
  const meta = await stat(agent);
  assert(agent.startsWith('/nix/store/') && meta.uid === 0 && (meta.mode & 0o222) === 0, 'Agent must be an immutable root-owned Nix-store executable');
  original = (await request(hostSocket, 'Workspaces')).Workspaces;
  const focused = original.find(w => w.is_focused);
  const test = original.find(w => w.name === 'dojo-tests' && !w.is_active)
    ?? original.find(w => !w.is_active && !w.name && w.active_window_id === null && w.output === focused?.output);
  assert(test, 'An inactive dojo-tests or empty workspace is required');
  const env = { ...process.env, DOJO_AGENT_INPUT: '1', DOJO_SEMANTIC_AGENT_EXE: agent, RUST_LOG: 'niri=info', ...(perception ? { DOJO_PERCEPTION_INSTALLATION: perception } : {}) };
  delete env.DOJO_BROWSER_CONTROL_BROKER_EXE; delete env.DOJO_BROWSER_CONTROL_SOCKET;
  delete env.AT_SPI_BUS_ADDRESS; delete env.DBUS_STARTER_ADDRESS; delete env.DBUS_STARTER_BUS_TYPE;
  const niri = await launch(niriBinary, ['--config', join(root, 'tools/agent-input/nested.kdl')], env, 'niri');
  const nested = await wait('nested compositor', async () => {
    if (niri.exitCode !== null || niri.failure) throw new Error('Nested compositor failed');
    const log = await readFile(join(evidence, 'niri.log'), 'utf8');
    const display = [...log.matchAll(/listening on Wayland socket: (\S+)/g)].at(-1)?.[1];
    const socket = [...log.matchAll(/IPC listening on: (\S+)/g)].at(-1)?.[1];
    return display && socket ? { display, socket } : undefined;
  });
  const host = await wait('nested host window', async () => (await request(hostSocket, 'Windows')).Windows.find(w => w.pid === niri.pid));
  await request(hostSocket, { Action: { MoveWindowToWorkspace: { window_id: host.id, reference: { Id: test.id }, focus: false } } });
  assert.deepEqual(active((await request(hostSocket, 'Workspaces')).Workspaces), active(original), 'Host active workspace changed');
  const nestedEnv = { ...env, WAYLAND_DISPLAY: nested.display, NIRI_SOCKET: nested.socket, GTK_A11Y: 'atspi', GDK_BACKEND: 'wayland' };
  const humanPointer = process.env.DOJO_TEST_HUMAN_POINTER ?? join(root, 'tools/agent-input/target/debug/human-pointer');
  try { await stat(humanPointer); } catch { execFileSync('cargo', ['build', '--manifest-path', join(root, 'tools/agent-input/Cargo.toml'), '--bin', 'human-pointer'], { stdio: 'inherit' }); }
  const logical = Object.values((await request(nested.socket, 'Outputs')).Outputs).find(output => output.logical)?.logical;
  assert(logical, 'Nested output logical dimensions unavailable');
  execFileSync(humanPointer, [String(logical.width - 2), String(logical.height - 2), String(logical.width), String(logical.height)], { env: nestedEnv });
  const fixtureBinary = join(evidence, 'gtk-fixture');
  execFileSync(process.env.DOJO_TEST_CC ?? 'cc', [join(root, 'tools/agent-input/gtk-accessibility-fixture.c'), '-o', fixtureBinary, '-ldl']);
  if (!gtk) {
    // Store closures differ in glibc; keep the first library this fixture can actually load.
    const candidates = readdirSync('/nix/store').filter(name => /-gtk4-[0-9.]+$/.test(name)).map(name => `/nix/store/${name}/lib/libgtk-4.so.1`);
    for (const candidate of candidates) {
      const probe = spawnSync(fixtureBinary, [candidate], { env: { ...nestedEnv, WAYLAND_DISPLAY: 'none' }, encoding: 'utf8', timeout: 5000 });
      if (!/GLIBC|cannot open shared object|undefined symbol/.test(probe.stderr ?? '')) { gtk = candidate; break; }
    }
    assert(gtk, 'No GTK4 library in /nix/store loads with this compiler; set DOJO_TEST_GTK_LIBRARY');
  }
  results.gtk = gtk;

  const SCRIPTS = {
    'gtk-press-save': [['Alpha Save', { kind: 'invoke' }]],
    'gtk-replace-text': [['Alpha Input', { kind: 'replace_text', text: 'Dojo' }]],
    'gtk-set-number': [['Alpha Value', { kind: 'set_numeric', value: 40 }]],
    'gtk-select-row': [['Alpha Choice Two', { kind: 'select' }]],
    'gtk-press-beta-save': [['Beta Save', { kind: 'invoke' }]],
  };
  const treeOf = async (client, windowId) => value(await call(client, 'desktop_accessibility', { window_id: windowId }));
  const startGtk = async () => {
    const app = await launch(fixtureBinary, [gtk], nestedEnv, `gtk-${results.runs.length}`);
    const windows = await wait('two GTK windows', async () => {
      if (app.exitCode !== null || app.failure) throw new Error('GTK fixture failed');
      const found = (await request(nested.socket, 'Windows')).Windows.filter(w => w.pid === app.pid);
      return found.length === 2 ? found : undefined;
    });
    const oracle = await DesktopMcp.open(root, { command: agent, env: nestedEnv, timeoutMs: 15000 }); clients.push(oracle);
    await request(nested.socket, 'YieldHumanInput');
    const byHint = await wait('exact Alpha and Beta trees', async () => {
      const found = {};
      for (const window of windows) {
        const tree = await treeOf(oracle, window.id);
        for (const hint of ['Alpha', 'Beta']) if (tree.nodes.some(n => n.name === `${hint} Save`)) found[hint] = window;
      }
      return found.Alpha && found.Beta && found.Alpha.id !== found.Beta.id ? found : undefined;
    });
    return { app, pid: app.pid, byHint, oracle };
  };
  const scriptedAgent = async (task, setup) => {
    const events = [];
    const mcp = await DesktopMcp.open(root, { command: agent, env: nestedEnv, timeoutMs: 15000 }); clients.push(mcp);
    try {
      for (const [name, action] of SCRIPTS[task.id]) {
        const alpha = name.startsWith('Alpha');
        const window = setup.byHint[alpha ? 'Alpha' : 'Beta'];
        const tree = await wait(`actionable ${name}`, async () => {
          const found = await treeOf(mcp, window.id);
          const node = found.nodes.find(n => n.name === name || n.name === name.replace('Value', 'Value'));
          return node && typeof found.element_refs?.[node.index] === 'string' ? { found, node } : undefined;
        });
        const route = 'desktop_accessibility_action';
        events.push({ kind: 'intent', route, data: { arguments: { action } } });
        const receipt = await call(mcp, route, { element_ref: tree.found.element_refs[tree.node.index], action });
        events.push({ kind: 'receipt', route, effect: 'unknown', data: { result: receipt.structuredContent ?? JSON.parse(receipt.content?.find(block => block.type === 'text')?.text ?? '{}'), is_error: receipt.isError === true } });
      }
    } finally { mcp.close(); }
    return events;
  };
  const dojoAgent = async task => {
    const [{ PiRuntime }, { exportDesktopTrace }] = await Promise.all([import('./pi-runtime.mjs'), import('./desktop-trace.mjs')]);
    const stateDir = await mkdtemp(join(evidence, 'pi-'));
    // The runtime reads a private copy of the one account credential; the live store is never opened for writing.
    if (process.env.DOJO_TEST_CREDENTIALS_FILE) await copyFile(process.env.DOJO_TEST_CREDENTIALS_FILE, join(stateDir, 'credentials.json'), 0);
    const { MemoryStorage } = await import('@earendil-works/pi-durable');
    Object.assign(process.env, { NIRI_SOCKET: nested.socket, WAYLAND_DISPLAY: nested.display, DOJO_SEMANTIC_AGENT_EXE: agent, ...(perception ? { DOJO_PERCEPTION_INSTALLATION: perception } : {}) });
    let idle; const done = new Promise(resolve => { idle = resolve; });
    let started = false;
    const runtime = await PiRuntime.open({ stateDir, root,
      memory: { async append() {}, async readyView() { return '<chat></chat>'; }, async zoom() { return ''; }, async date() { return new Date().toISOString(); } },
      config: { storage: new MemoryStorage(), browserTools: false, ...(explicitModel ?? { modelSelection: liveSelection }) },
      onEvent: event => { if (event.type === 'run' && event.status === 'running') started = true; if (event.type === 'run' && (event.status === 'idle' || event.status === 'error') && started) idle(event); } });
    try {
      await runtime.submit(task.goal);
      const timeout = Number(option('--task-timeout-ms') ?? 180000);
      const outcome = await Promise.race([done, delay(timeout).then(() => ({ status: 'timeout' }))]);
      if (outcome.status === 'timeout') await runtime.stop();
      await runtime.archived;
      const trace = await runtime.exportDesktopTrace();
      // Bounded transcript for diagnosis: roles, stop reasons, errors and short text only.
      const page = await runtime.conversation.entries({ order: 'ascending' }, 200, undefined, runtime.context);
      const transcript = page.items.map(entry => {
        const message = entry.model?.[0] ?? {};
        const text = (Array.isArray(message.content) ? message.content : []).map(block => block.type === 'text' ? block.text : `[${block.type}${block.name ? ` ${block.name}` : ''}]`).join(' ');
        return { kind: entry.kind, stop: message.stopReason, error: message.errorMessage, text: (text || (typeof message.content === 'string' ? message.content : JSON.stringify(entry.model ?? entry.data ?? entry))).slice(0, 400) };
      });
      return { events: trace.events, ended: outcome.status, transcript };
    } finally { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); }
  };


  // Stock Electron runs the repository's pointer fixture. Its page logs mouse events to stdout, which is
  // the oracle: the fixture has no accessibility route by decision, so success is read from its own log.
  const startElectron = async () => {
    const binary = process.env.DOJO_TEST_ELECTRON_BINARY ?? join(homedir(), '.local/opt/t3code/t3code');
    await stat(binary).catch(() => assert.fail(`Electron binary not found: ${binary}; set DOJO_TEST_ELECTRON_BINARY`));
    const runtimeDir = join(evidence, `electron-${results.runs.length}`);
    await mkdir(join(runtimeDir, 'resources/app'), { recursive: true });
    const source = await realpath(binary);
    for (const name of await readdir(join(source, '..'))) {
      if (name === 'resources') continue;
      const target = join(runtimeDir, name);
      if (join(source, '..', name) === source) await copyFile(source, target); else await symlink(join(source, '..', name), target);
    }
    await writeFile(join(runtimeDir, 'resources/app/package.json'), JSON.stringify({ name: 'dojo-seat-fixture', version: '1.0.0', main: 'main.cjs' }));
    await copyFile(join(root, 'tools/agent-input/electron-fixture.cjs'), join(runtimeDir, 'resources/app/main.cjs'));
    const electronEnv = { ...nestedEnv, DOJO_ELECTRON_PROFILE: join(runtimeDir, 'profile') };
    delete electronEnv.ELECTRON_RUN_AS_NODE; delete electronEnv.LD_LIBRARY_PATH; delete electronEnv.LD_PRELOAD;
    const label = `electron-${results.runs.length}`;
    const app = await launch(join(runtimeDir, basename(source)), ['--ozone-platform=wayland', '--no-first-run'], electronEnv, label);
    const logPath = join(evidence, `${label}.log`);
    const window = await wait('Electron fixture window', async () => {
      if (app.exitCode !== null || app.failure) throw new Error('Electron fixture failed');
      const found = (await request(nested.socket, 'Windows')).Windows.find(w => w.pid === app.pid && w.title === 'Dojo Electron seat fixture');
      return found && (await readFile(logPath, 'utf8').catch(() => '')).includes('FIXTURE_READY') ? found : undefined;
    }, 60000);
    await delay(500);
    const oracle = await DesktopMcp.open(root, { command: agent, env: nestedEnv, timeoutMs: 15000 }); clients.push(oracle);
    await request(nested.socket, 'YieldHumanInput');
    return { app, pid: app.pid, window, logPath, oracle, output: Object.keys((await request(nested.socket, 'Outputs')).Outputs)[0] };
  };
  const scriptedVision = async setup => {
    const events = [];
    const mcp = await DesktopMcp.open(root, { command: agent, env: nestedEnv, timeoutMs: 15000 }); clients.push(mcp);
    try {
      const observed = value(await call(mcp, 'desktop_observe', { output: setup.output, window_id: setup.window.id }));
      events.push({ kind: 'intent', route: 'desktop_observe', data: { arguments: {} } }, { kind: 'receipt', route: 'desktop_observe', effect: 'unknown', data: { result: { capture_id: observed.capture_id }, is_error: false } });
      if (perception) {
        // Exercise OCR through the MCP on the same capture the click uses; it must read the fixture's button label.
        const parsed = await call(mcp, 'parse_visual_regions', { capture_id: observed.capture_id });
        const body = parsed.structuredContent ?? JSON.parse(parsed.content.find(block => block.type === 'text').text);
        assert.equal(parsed.isError ?? false, false, `parse_visual_regions failed: ${JSON.stringify(body).slice(0, 300)}`);
        const texts = []; const walk = node => { if (node && typeof node === 'object') for (const [key, child] of Object.entries(node)) { if (key === 'text' && typeof child === 'string') texts.push(child); else walk(child); } };
        walk(body);
        results.perception_check = { configured: perception, text_regions: texts.length, sample: texts.slice(0, 5) };
        assert(texts.some(text => /click/i.test(text)), `OCR did not read the button label: ${JSON.stringify(texts.slice(0, 8))}`);
        events.push({ kind: 'intent', route: 'parse_visual_regions', data: { arguments: {} } }, { kind: 'receipt', route: 'parse_visual_regions', effect: 'unknown', data: { result: { text_regions: texts.length }, is_error: false } });
      }
      const input = { kind: 'click', button: 'left' };
      const action = { capture_id: observed.capture_id, x: Math.round(observed.image_width / 2), y: Math.round(observed.image_height / 2), input, observe_after: false };
      events.push({ kind: 'intent', route: 'desktop_capture_action', data: { arguments: action } });
      const receipt = await call(mcp, 'desktop_capture_action', action);
      events.push({ kind: 'receipt', route: 'desktop_capture_action', effect: 'unknown', data: { result: receipt.structuredContent ?? JSON.parse(receipt.content?.find(block => block.type === 'text')?.text ?? '{}'), is_error: receipt.isError === true } });
    } finally { mcp.close(); }
    return events;
  };
  const checkOracle = async (setup, check) => {
    if (typeof check.fixture_log_contains === 'string') {
      await delay(500);
      return (await readFile(setup.logPath, 'utf8')).includes(check.fixture_log_contains) ? 'satisfied' : 'unsatisfied';
    }
    const window = setup.byHint[check.window];
    assert(window, `No window for hint ${check.window}`);
    return value(await call(setup.oracle, 'desktop_verify', { pid: setup.pid, window_id: window.id, timeout_ms: 3000, expect: check.expect })).status;
  };
  for (const task of selected) {
    if (!RUNNABLE.has(task.fixture)) {
      const hostBound = ['host-desktop', 'helium'].includes(task.fixture);
      results.runs.push(skipped(task, hostBound ? `fixture ${task.fixture} needs the host browser profile; deliberately not run in the isolated workspace` : `fixture ${task.fixture} has no live adapter`));
      continue;
    }
    let setup;
    try {
      setup = await (task.fixture === 'gtk' ? startGtk() : startElectron());
      // A dropped provider connection says nothing about the agent: retry once on a fresh fixture state, then report it.
      let piRun, providerError, attempts = 0;
      if (agentKind === 'dojo') {
        do {
          attempts += 1;
          piRun = await dojoAgent(task);
          providerError = piRun.transcript.find(entry => entry.stop === 'error')?.error;
          if (providerError && attempts <= Number(option('--agent-retries') ?? 1)) {
            setup.oracle.close(); await stop(setup.app);
            setup = await (task.fixture === 'gtk' ? startGtk() : startElectron());
          }
        } while (providerError && attempts <= Number(option('--agent-retries') ?? 1));
      }
      if (providerError) { results.transcripts ??= {}; results.transcripts[task.id] = piRun.transcript; results.runs.push({ ...agentError(task, providerError), attempts }); continue; }
      const events = agentKind === 'dojo' ? piRun.events : task.fixture === 'gtk' ? await scriptedAgent(task, setup) : await scriptedVision(setup);
      const statuses = [];
      for (const check of task.success) statuses.push(await checkOracle(setup, check));
      results.runs.push({ ...scoreRun(task, events, statuses), ...(piRun ? { ended: piRun.ended, attempts } : {}) });
      results.evidence_events ??= {}; results.evidence_events[task.id] = events;
      if (piRun?.transcript) { results.transcripts ??= {}; results.transcripts[task.id] = piRun.transcript; }
    } catch (error) {
      results.runs.push({ task_id: task.id, tier: task.tier, outcome: 'error', error: String(error.message ?? error) });
    } finally {
      if (setup) { setup.oracle.close(); await stop(setup.app); }
    }
  }
  const scored = results.runs.filter(run => run.outcome !== 'error');
  results.summary = summarize(scored);
  results.errors = results.runs.filter(run => run.outcome === 'error').length;
  const final = (await request(hostSocket, 'Workspaces')).Workspaces;
  assert.deepEqual(active(final), active(original), 'Host active workspace changed');
  results.host_workspace_unchanged = true;
} finally {
  await writeFile(join(evidence, 'results.json'), JSON.stringify(results, null, 2));
  for (const client of clients) client.close();
  for (const child of children.reverse()) await stop(child);
  await Promise.all(logs.map(log => log.close()));
}
console.log(JSON.stringify({ summary: results.summary, errors: results.errors, runs: results.runs.map(({ task_id, outcome, actions, reason, error }) => ({ task_id, outcome, actions, reason, error })) }, null, 1));
// A failed runtime open can leave handles that keep the process alive; the results are already written.
process.exit(results.errors ? 1 : 0);
