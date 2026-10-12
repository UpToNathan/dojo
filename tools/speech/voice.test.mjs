import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { once } from 'node:events';

const directory = await mkdtemp(join(tmpdir(), 'dojo-voice-test-'));
process.env.XDG_RUNTIME_DIR = directory;
process.env.XDG_STATE_HOME = directory;
const { VoiceService, serve, acquireServerLock, socketRequest } = await import('./voice.mjs');
const { T3Client } = await import('./t3-client.mjs');
const statePath = join(directory, 'dojo/voice/conversations.json');
const tick = () => new Promise(resolve => setImmediate(resolve));
function service() {
  const result = new VoiceService();
  result.state = { version: 1, activeThreadId: null, chats: [], seenMessages: [] };
  result.config = {};
  result.ui = async () => {};
  return result;
}
function chat(result) {
  const value = { threadId: 'thread', listenSince: '2026-01-01', activeRunId: null };
  result.state.chats.push(value);
  result.state.activeThreadId = value.threadId;
  return value;
}
function backend() {
  const subscriptions = [];
  return {
    socket: { readyState: WebSocket.OPEN }, subscriptions, interrupted: [], closed: false,
    subscribe(threadId, onValue, afterSequence) {
      let resolve, reject;
      const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
      const value = { id: String(subscriptions.length), promise, threadId, onValue, afterSequence, resolve, reject };
      subscriptions.push(value); return value;
    },
    unsubscribe(id) { this.interrupted.push(id); },
    async call() {}, close() { this.closed = true; },
  };
}
async function request(payload) {
  const socket = net.createConnection(join(directory, 'dojo-voice.sock'));
  await once(socket, 'connect');
  socket.write(payload);
  let response = '';
  for await (const data of socket) response += data;
  return response;
}

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

test('follow-ups keep one subscription and reconnect requests sequence replay', async () => {
  const result = service(); const current = chat(result); current.sequence = 17;
  let client = backend(); result.connect = async () => client;
  await result.watch(); await result.watch();
  assert.equal(client.subscriptions.length, 1);
  assert.equal(client.subscriptions[0].afterSequence, 17);
  const previous = client;
  client = backend();
  await result.watch();
  assert.deepEqual(previous.interrupted, ['0']);
  assert.equal(client.subscriptions[0].afterSequence, 17);
  await result.shutdown();
});

test('expired sequence replay falls back to a snapshot subscription', async () => {
  const result = service(); const current = chat(result); current.sequence = 17;
  const client = backend(); result.client = client; result.connect = async () => client;
  await result.watch();
  client.subscriptions[0].reject(new Error('Replay history expired'));
  await tick();
  assert.equal(client.subscriptions.length, 2);
  assert.equal(client.subscriptions[1].afterSequence, undefined);
  await result.shutdown();
});

test('streaming updates coalesce writes and shutdown persists the last sequence', async () => {
  const result = service(); const current = chat(result);
  let writes = 0;
  const save = result.save.bind(result);
  result.save = () => { writes++; return save(); };
  for (let sequence = 1; sequence <= 100; sequence++) await result.stream(current, {
    kind: 'event', sequence, event: { type: 'message.updated', payload: { role: 'assistant', streaming: true } },
  });
  assert.equal(writes, 0);
  await result.shutdown();
  assert.equal(writes, 1);
  assert.equal(JSON.parse(await readFile(statePath, 'utf8')).chats[0].sequence, 100);
});

test('completed assistant messages persist and display once while duplicate sequence is ignored', async () => {
  const result = service(); const current = chat(result); const displayed = [];
  result.ui = async (event, text) => displayed.push({ event, text });
  const item = { kind: 'event', sequence: 1, event: { type: 'message.updated', payload: {
    role: 'assistant', id: 'message', streaming: false, text: 'Ready.', createdAt: '2026-10-11',
  } } };
  await result.stream(current, item); await result.stream(current, item);
  assert.deepEqual(displayed, [{ event: 'response', text: 'Ready.' }]);
  assert.deepEqual(JSON.parse(await readFile(statePath, 'utf8')).seenMessages, ['message']);
  await result.shutdown();
});

test('settlement uses a single deadline, waits for capture and closes the backend', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 0 });
  const result = service(); const current = chat(result); const client = backend();
  result.client = client; result.connect = async () => client;
  result.capture = { id: 'recording' }; result.scheduleDeadline();
  assert.equal(result.deadlineTimer, undefined);
  t.mock.timers.tick(40_000); assert.equal(result.active(), current);
  result.capture = null; result.touch();
  t.mock.timers.tick(29_999); await tick(); assert.equal(result.active(), current);
  t.mock.timers.tick(1); await result.queue; await tick();
  assert.equal(result.state.activeThreadId, null);
  assert.equal(client.closed, true);
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

test('T3 acknowledges chunks only after their consumer processes them', async () => {
  const client = new T3Client(); const frames = [];
  client.socket = { send: text => frames.push(JSON.parse(text)), readyState: WebSocket.OPEN };
  let release; const gate = new Promise(resolve => { release = resolve; });
  const consumed = [];
  const subscription = client.subscribe('thread', async value => { await gate; consumed.push(value); });
  const receiving = client.receive({ _tag: 'Chunk', requestId: subscription.id, values: ['first', 'second'] });
  await tick(); assert.equal(frames.some(frame => frame._tag === 'Ack'), false);
  release(); await receiving;
  assert.deepEqual(consumed, ['first', 'second']);
  assert.deepEqual(frames.at(-1), { _tag: 'Ack', requestId: subscription.id });
  client.unsubscribe(subscription.id); await subscription.promise;
  assert.equal(client.pending.size, 0);
});

test.after(async () => { await rm(directory, { recursive: true, force: true }); });

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

test('reconnect owns one timer and backs off repeated backend failures', async t => {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 0 });
  const result = service(); chat(result).activeRunId = 'run';
  let attempts = 0;
  result.watch = async () => { attempts++; if (attempts < 3) throw new Error('Stub disconnected backend'); };
  result.scheduleReconnect(); const first = result.reconnectTimer;
  result.scheduleReconnect(); assert.equal(result.reconnectTimer, first);
  t.mock.timers.tick(1000); await tick(); assert.equal(attempts, 1);
  t.mock.timers.tick(1999); await tick(); assert.equal(attempts, 1);
  t.mock.timers.tick(1); await tick(); assert.equal(attempts, 2);
  t.mock.timers.tick(4000); await tick(); assert.equal(attempts, 3);
  assert.equal(result.reconnecting, false);
  assert.equal(result.reconnectTimer, undefined);
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
