import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';
import { spawnSync } from 'node:child_process';

const directory = await mkdtemp(join(tmpdir(), 'dojo-voice-test-'));
process.env.XDG_RUNTIME_DIR = directory;
process.env.XDG_STATE_HOME = directory;
const { VoiceService, serve, acquireServerLock, socketRequest } = await import('./voice.mjs');
const statePath = join(directory, 'dojo/voice/conversations.json');
const tick = () => new Promise(resolve => setImmediate(resolve));
function service() {
  const result = new VoiceService();
  result.state = { version: 1, activeThreadId: null, chats: [], seenMessages: [] };
  result.config = {};
  result.ui = async () => {};
  result.microphoneCheck = async () => true;
  return result;
}
function serviceWithFakeSpeechWorker() {
  const result = service(), commands = [];
  result.microphone = true;
  result.speechWorker = { stdin: { write(line) { commands.push(JSON.parse(line)); }, end() {} } };
  return { result, commands };
}
function chat(result) {
  const value = { threadId: 'thread', listenSince: '2026-01-01', activeRunId: null };
  result.state.chats.push(value);
  result.state.activeThreadId = value.threadId;
  return value;
}
async function request(payload) {
  const socket = net.createConnection(join(directory, 'dojo-voice.sock'));
  await once(socket, 'connect');
  socket.write(payload);
  let response = '';
  for await (const data of socket) response += data;
  return response;
}

test('trace command exports the current runtime without changing its activity or chat', async () => {
  const result = service();
  const current = chat(result);
  const activity = result.lastActivity;
  const trace = { schema_version: 'dojo.computer.trace/1', events: [] };
  result.runtime = { exportDesktopTrace: async () => trace };
  assert.equal(await result.handle({ action: 'trace' }), trace);
  assert.equal(result.active(), current);
  assert.equal(result.lastActivity, activity);
});

test('trace CLI does not start a daemon when the service is absent', async () => {
  const runtimeDirectory = await mkdtemp(join(tmpdir(), 'dojo-trace-offline-'));
  try {
    const child = spawnSync(process.execPath, [new URL('./voice.mjs', import.meta.url).pathname, 'trace'], {
      env: { ...process.env, XDG_RUNTIME_DIR: runtimeDirectory, XDG_STATE_HOME: runtimeDirectory }, encoding: 'utf8',
    });
    assert.equal(child.status, 1, child.stderr);
    assert.match(JSON.parse(child.stdout).error, /service is not running/);
    await assert.rejects(readFile(join(runtimeDirectory, 'dojo/voice/voice.log')), { code: 'ENOENT' });
  } finally { await rm(runtimeDirectory, { recursive: true, force: true }); }
});

test('trace export failure does not mark the agent task as failed', async () => {
  const result = service();
  let failures = 0;
  result.start = async () => {};
  result.failure = async () => { failures++; };
  result.runtime = { exportDesktopTrace: async () => { throw new Error('Trace limit exceeded'); }, close: async () => {} };
  const server = await serve(result);
  try {
    assert.deepEqual(JSON.parse(await request('{"action":"trace"}\n')), { error: 'Trace limit exceeded' });
    assert.equal(failures, 0);
  } finally { await server.shutdown(); }
});

test('every server launch takes the same lock before preload and socket replacement', async () => {
  const first = service(), second = service();
  let preloads = 0;
  first.start = second.start = async () => { preloads++; };
  const [a, b] = await Promise.all([serve(first), serve(second)]);
  assert.equal(preloads, 1);
  const owner = a || b;
  try {
    assert.deepEqual(JSON.parse(await request('{"action":"status"}\n')), {
      recording: false, transcribing: false, microphone: null, threadId: null, runId: null,
    });
    assert.deepEqual(JSON.parse(await request('{bad}\n')), { error: 'Invalid command' });
    assert.equal(await request('a'.repeat(16_385)), '');
  } finally { await owner.shutdown(); }
  const lock = await acquireServerLock();
  assert.ok(lock);
  lock.stdin.end(); await once(lock, 'exit');
});

test('failed startup releases singleton ownership', async () => {
  const result = service();
  result.start = async () => { throw new Error('stub startup failure'); };
  await assert.rejects(serve(result), /stub startup failure/);
  await tick();
  const lock = await acquireServerLock();
  assert.ok(lock);
  lock.stdin.end(); await once(lock, 'exit');
});

test('bubble settlement waits for capture without closing the continuous runtime', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 0 });
  const result = service(); const current = chat(result);
  result.capture = { id: 'recording' }; result.scheduleDeadline();
  assert.equal(result.deadlineTimer, undefined);
  t.mock.timers.tick(40_000); assert.equal(result.active(), current);
  result.capture = null; result.touch();
  t.mock.timers.tick(29_999); await tick(); assert.equal(result.active(), current);
  t.mock.timers.tick(1); await result.queue; await tick();
  assert.equal(result.state.activeThreadId, null);
  assert.equal(result.deadlineTimer, undefined);
  await result.shutdown();
});

test('idle exit runs outside the command queue and completes its shutdown flush', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 0 });
  const result = service(); result.idleSeconds = 1;
  let exited = false;
  result.onIdle = async () => { await result.shutdown(); exited = true; };
  result.scheduleDeadline(); t.mock.timers.tick(1000);
  await result.queue; await tick(); await result.saving; await tick();
  assert.equal(exited, true);
});

test('command queue is bounded without dropping accepted operations', async () => {
  const result = service(); let release; const gate = new Promise(resolve => { release = resolve; });
  const operations = Array.from({ length: 128 }, (_, index) => result.enqueue(async () => { await gate; return index; }));
  await assert.rejects(result.enqueue(async () => 129), /queue is busy/);
  release();
  assert.deepEqual(await Promise.all(operations), Array.from({ length: 128 }, (_, index) => index));
  await result.shutdown();
});

test('voice preserves exact request text and keeps the durable conversation after bubble expiry', async () => {
  const result = service(), received = [];
  result.runtime = { id: 1, busy: () => false, submit: async text => { received.push(text); return {}; }, close: async () => {} };
  await result.submit('  preserve this\n');
  assert.deepEqual(received, ['  preserve this\n']);
  assert.equal(result.state.activeThreadId, '1');
  await result.runtimeEvent({ type: 'run', status: 'idle', conversationId: 1 });
  await result.settle(true);
  assert.equal(result.state.activeThreadId, null);
  assert.equal(result.state.chats[0].threadId, '1');
  await result.shutdown();
});

test('native worker events preserve warm reuse, capture cancellation and stale-result rejection', async () => {
  const { EventEmitter } = await import('node:events');
  const { PassThrough } = await import('node:stream');
  const result = service(); const ui = []; const commands = [];
  result.ui = async (event, value) => ui.push({ event, value });
  let spawns = 0;
  const child = new EventEmitter();
  child.stdout = new PassThrough(); child.stderr = new PassThrough(); child.stdin = new PassThrough();
  child.stdin.on('data', data => commands.push(JSON.parse(data.toString())));
  result.spawnWorker = () => { spawns++; return child; };
  await result.toggle();
  const first = result.capture.id;
  child.stdout.write(JSON.stringify({ event: 'listening', id: first, device: 'Stub microphone' }) + '\n');
  assert.equal(result.captureDevice, 'Stub microphone');
  await result.finish();
  child.stdout.write(JSON.stringify({ event: 'transcribing', id: first }) + '\n');
  child.stdout.write(JSON.stringify({ event: 'transcript', id: first, text: '', inferenceMs: 0 }) + '\n');
  await result.queue;
  await result.toggle();
  const second = result.capture.id;
  await result.cancel();
  child.stdout.write(JSON.stringify({ event: 'transcript', id: second, text: 'Must not dispatch' }) + '\n');
  await result.queue;
  assert.equal(spawns, 1);
  assert.deepEqual(commands.map(({ action }) => action), ['begin', 'finish', 'begin', 'cancel']);
  assert.equal(result.capture, null);
  assert.equal(ui.some(({ event, value }) => event === 'phase' && value === 'transcribing'), true);
  assert.equal(ui.some(({ event }) => event === 'transcript'), false);
  await result.shutdown(); child.stdout.end(); child.stderr.end();
});

test('controller begin leaves a keyboard-owned capture unchanged', async () => {
  const { result, commands } = serviceWithFakeSpeechWorker();
  const capture = { id: 'keyboard-capture' };
  result.capture = capture;
  result.recording = true;

  const reply = await result.handle({ action: 'controller-begin', owner: '4b43d7be-e2f7-4d91-a08e-0e6741e65b92' });

  assert.deepEqual(reply, { recording: true, owned: false, busy: true });
  assert.equal(result.capture, capture);
  assert.equal(result.recording, true);
  assert.deepEqual(commands, []);
});

test('controller ownership is atomic, idempotent, and required to finish or cancel', async () => {
  const { result, commands } = serviceWithFakeSpeechWorker();
  const owner = '4b43d7be-e2f7-4d91-a08e-0e6741e65b92';
  const other = '5c54e8cf-f308-4ea2-b19f-1f7852f76ca3';

  const first = await result.handle({ action: 'controller-begin', owner });
  const capture = result.capture;
  const duplicate = await result.handle({ action: 'controller-begin', owner });
  assert.deepEqual(first, { recording: true, owned: true, busy: false });
  assert.deepEqual(duplicate, { recording: true, owned: true, busy: false });
  assert.equal(result.capture, capture);
  assert.deepEqual(commands.map(({ action }) => action), ['begin']);

  assert.deepEqual(await result.handle({ action: 'controller-finish', owner: other }), { recording: true, owned: false });
  assert.deepEqual(await result.handle({ action: 'controller-cancel', owner: other }), { recording: true, owned: false });
  assert.equal(result.capture, capture);
  assert.equal(result.recording, true);
  assert.deepEqual(commands.map(({ action }) => action), ['begin']);

  assert.deepEqual(await result.handle({ action: 'controller-finish', owner }), { recording: false, owned: true });
  assert.deepEqual(commands.map(({ action }) => action), ['begin', 'finish']);
  await result.shutdown();
});

test('controller cancellation is limited to the owner and invalid owner IDs are rejected', async () => {
  const { result, commands } = serviceWithFakeSpeechWorker();
  const owner = '4b43d7be-e2f7-4d91-a08e-0e6741e65b92';

  await assert.rejects(result.handle({ action: 'controller-begin', owner: '../not-a-uuid' }), /owner must be a UUID/);
  await result.handle({ action: 'controller-begin', owner });
  assert.deepEqual(await result.handle({ action: 'controller-cancel', owner }), { recording: false, owned: true });
  assert.equal(result.capture, null);
  assert.deepEqual(commands.map(({ action }) => action), ['begin', 'cancel']);
  await result.shutdown();
});

test('unexpected lock-holder death closes the server, clients and native input', async () => {
  const result = service(); let inputClosed = false;
  result.start = async () => { result.speechWorker = { stdin: { end() { inputClosed = true; } } }; };
  const owner = await serve(result);
  const socket = net.createConnection(join(directory, 'dojo-voice.sock'));
  await once(socket, 'connect');
  const clientClosed = once(socket, 'close');
  owner.lock.kill('SIGKILL');
  await once(owner.lock, 'exit');
  await owner.shutdown(); await clientClosed;
  assert.equal(inputClosed, true);
  assert.equal(result.closing, true);
  await assert.rejects(request('{"action":"status"}\n'), error => ['ENOENT', 'ECONNREFUSED'].includes(error.code));
  const replacement = service(); replacement.start = async () => {};
  const next = await serve(replacement);
  assert.ok(next); await next.shutdown();
});

test('Unix socket admission caps simultaneous clients at 32', async () => {
  const result = service(); result.start = async () => {};
  const owner = await serve(result); const sockets = [];
  try {
    for (let index = 0; index < 32; index++) {
      const socket = net.createConnection(join(directory, 'dojo-voice.sock'));
      sockets.push(socket); await once(socket, 'connect');
    }
    const extra = net.createConnection(join(directory, 'dojo-voice.sock'));
    const closed = once(extra, 'close');
    await once(extra, 'connect'); await closed;
    sockets[0].write('{"action":"status"}\n');
    assert.equal(JSON.parse((await once(sockets[0], 'data'))[0].toString()).recording, false);
  } finally {
    for (const socket of sockets) socket.destroy();
    await owner.shutdown();
  }
});


test('voice commands preserve Unicode when UTF-8 bytes cross socket chunks', async () => {
  const result = service(); result.start = async () => {};
  result.handle = async command => ({ text: command.text });
  const owner = await serve(result);
  const accepted = once(owner.server, 'connection');
  const socket = net.createConnection(join(directory, 'dojo-voice.sock'));
  socket.setEncoding('utf8');
  const [incoming] = await accepted;
  const payload = Buffer.from(JSON.stringify({ action: 'send', text: '你好 🍵 café' }) + '\n');
  const split = payload.indexOf(Buffer.from('你')) + 1;
  const receivedFirstChunk = once(incoming, 'data');
  socket.write(payload.subarray(0, split));
  await receivedFirstChunk;
  socket.write(payload.subarray(split));
  let response = '';
  try {
    for await (const data of socket) response += data;
    assert.deepEqual(JSON.parse(response), { text: '你好 🍵 café' });
  } finally { socket.destroy(); await owner.shutdown(); }
});

test('outgoing socket responses preserve Unicode split across UTF-8 chunks', async () => {
  const path = join(directory, 'unicode-response.sock');
  const expected = JSON.stringify({ text: '你好 🍵 café' });
  const payload = Buffer.from(expected + '\n');
  const split = payload.indexOf(Buffer.from('🍵')) + 2;
  const server = net.createServer(socket => {
    socket.once('data', () => {
      socket.write(payload.subarray(0, split));
      setTimeout(() => socket.end(payload.subarray(split)), 20);
    });
  });
  await new Promise(resolve => server.listen(path, resolve));
  try { assert.equal(await socketRequest(path, { action: 'status' }), expected); }
  finally { await new Promise(resolve => server.close(resolve)); }
});

test('command admission keeps its UTF-16 code-unit bound rather than truncating Unicode bytes', async () => {
  const result = service(); result.start = async () => {};
  result.handle = async command => ({ text: command.text });
  const owner = await serve(result);
  const text = '你'.repeat(6_000);
  const payload = JSON.stringify({ action: 'send', text }) + '\n';
  assert.ok(Buffer.byteLength(payload) > 16_384);
  assert.ok(payload.length < 16_384);
  try { assert.deepEqual(JSON.parse(await socketRequest(join(directory, 'dojo-voice.sock'), { action: 'send', text })), { text }); }
  finally { await owner.shutdown(); }
});

function fakeWorker() {
  const stdin = new PassThrough(), commands = [];
  stdin.on('data', data => commands.push(JSON.parse(data.toString())));
  const child = Object.assign(new EventEmitter(), { stdout: new PassThrough(), stderr: new PassThrough(), stdin });
  child.stdin.ended = false;
  stdin.on('end', () => { child.stdin.ended = true; });
  return { child, commands };
}
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';

test('agent commands run with no microphone and never start the speech worker', async () => {
  const result = service(); let spawns = 0;
  result.microphoneCheck = async () => false;
  result.spawnWorker = () => { spawns++; throw new Error('speech worker must not start'); };
  const submitted = [];
  result.runtime = { id: 'conversation', busy: () => false, submit: async text => { submitted.push(text); return { ok: true }; },
    respondHuman: async () => {}, stop: async () => {}, close: async () => {}, modelRows: [], accounts: [], reloadAccountCatalog: async () => {} };
  assert.deepEqual(await result.handle({ action: 'send', text: 'hello' }), { threadId: 'conversation' });
  assert.deepEqual(submitted, ['hello']);
  assert.equal((await result.handle({ action: 'history' })).chats.length, 1);
  assert.deepEqual((await result.handle({ action: 'models' })).models, []);
  result.state.chats[0].activeRunId = null;
  await result.handle({ action: 'new' });
  await result.handle({ action: 'stop' });
  await result.handle({ action: 'cancel' });
  await result.handle({ action: 'finish' });
  assert.equal((await result.handle({ action: 'status' })).recording, false);
  assert.equal(spawns, 0);
  assert.equal(result.speechWorker, undefined);
  await result.shutdown();
});

test('voice actions say there is no microphone instead of starting the worker', async () => {
  const result = service(); let spawns = 0;
  result.microphoneCheck = async () => false;
  result.spawnWorker = () => { spawns++; throw new Error('speech worker must not start'); };
  const owner = '4b43d7be-e2f7-4d91-a08e-0e6741e65b92';
  await assert.rejects(result.handle({ action: 'toggle' }), /No microphone is available/);
  await assert.rejects(result.handle({ action: 'controller-begin', owner }), /No microphone is available/);
  assert.equal(result.capture, undefined);
  assert.equal(spawns, 0);
  // A microphone that appears without the watcher noticing is found on the next keypress.
  result.microphoneCheck = async () => true;
  const { child } = fakeWorker();
  result.spawnWorker = () => { spawns++; return child; };
  await result.handle({ action: 'toggle' });
  assert.equal(spawns, 1);
  assert.ok(result.capture);
  await result.shutdown();
});

test('voice-input follows microphone hotplug and touches only the speech worker', async () => {
  const result = service(); let spawns = 0;
  const { child, commands } = fakeWorker();
  result.spawnWorker = () => { spawns++; return child; };
  const runtime = { id: 'conversation', busy: () => false, stop: async () => {}, close: async () => {} };
  result.runtime = runtime;
  await assert.rejects(result.handle({ action: 'voice-input' }), /available: true or false/);
  assert.deepEqual(await result.handle({ action: 'voice-input', available: true }), { microphone: true });
  await result.handle({ action: 'voice-input', available: true });
  assert.equal(spawns, 1);
  await result.handle({ action: 'toggle' });
  assert.ok(result.capture);
  // Losing the microphone mid-capture cancels it, ends the worker and leaves the runtime alone.
  result.microphoneCheck = async () => false;
  assert.deepEqual(await result.handle({ action: 'voice-input', available: false }), { microphone: false });
  assert.deepEqual(commands.map(({ action }) => action), ['begin', 'cancel']);
  assert.equal(result.capture, null);
  assert.equal(result.speechWorker, null);
  assert.equal(child.stdin.writableEnded, true);
  assert.equal(result.runtime, runtime);
  await assert.rejects(result.handle({ action: 'toggle' }), /No microphone is available/);
  await result.shutdown();
});

test('a server without a microphone starts the agent runtime and answers text commands', async () => {
  const result = service(); let spawns = 0;
  result.microphoneCheck = async () => false;
  result.spawnWorker = () => { spawns++; throw new Error('speech worker must not start'); };
  result.state = undefined;
  result.openRuntime = async () => ({ id: 'conversation', busy: () => false, resume() {}, scheduleCompaction() {}, close: async () => {},
    accounts: [], modelRows: [], reloadAccountCatalog: async () => {} });
  const owner = await serve(result);
  try {
    assert.ok(result.runtime, 'the agent runtime is open');
    assert.ok(!result.speechWorker);
    assert.equal(JSON.parse(await request('{"action":"status"}\n')).recording, false);
    assert.deepEqual(JSON.parse(await request('{"action":"history"}\n')).chats.map(chat => chat.threadId), ['conversation']);
    const catalog = JSON.parse(await request('{"action":"models"}\n'));
    assert.ok(Array.isArray(catalog.models) && Array.isArray(catalog.accounts));
    assert.match(JSON.parse(await request('{"action":"toggle"}\n')).error, /No microphone is available/);
    assert.equal(spawns, 0);
  } finally { await owner.shutdown(); }
});

test('a runtime started by systemd listens on the socket systemd passes it, and only then', async () => {
  const { inheritedSocket } = await import('./voice.mjs');
  assert.deepEqual(inheritedSocket({ LISTEN_PID: '42', LISTEN_FDS: '1' }, 42), { fd: 3 });
  assert.equal(inheritedSocket({}, 42), null, 'started directly');
  assert.equal(inheritedSocket({ LISTEN_PID: '41', LISTEN_FDS: '1' }, 42), null, 'the variables were meant for another process');
  assert.equal(inheritedSocket({ LISTEN_PID: '42', LISTEN_FDS: '2' }, 42), null, 'more than the one socket this runtime expects');
  assert.equal(inheritedSocket({ LISTEN_PID: '42' }, 42), null);
});

test('the acknowledge command reaches the runtime and refuses when no browser extension is loaded', async () => {
  const result = service();
  chat(result);
  result.runtime = {};
  await assert.rejects(result.handle({ action: 'acknowledge-browser' }), /browser extension is not loaded/);
  result.runtime = { acknowledgeBrowser: async () => ({ cleared: true }) };
  assert.deepEqual(await result.handle({ action: 'acknowledge-browser' }), { cleared: true });
  result.runtime = {};
  await assert.rejects(result.handle({ action: 'browser-status' }), /browser extension is not loaded/);
  result.runtime = { browserStatus: async () => ({ uncertain: false }) };
  assert.deepEqual(await result.handle({ action: 'browser-status' }), { uncertain: false });
});
