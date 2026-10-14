import net from 'node:net';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, unlink, chmod, open, stat } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { dataDir, readJson, saveJson } from './voice-state.mjs';
import { MemoryStore } from './memory.mjs';
import { PiRuntime } from './pi-runtime.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const runtimeDir = process.env.XDG_RUNTIME_DIR;
if (!runtimeDir) throw new Error('XDG_RUNTIME_DIR is required.');
const socketPath = join(runtimeDir, 'dojo-voice.sock');
const heldPath = join(runtimeDir, 'dojo-voice-held');
const statePath = join(dataDir, 'conversations.json');
const maxCommandCodeUnits = 16_384;
const terminal = new Set(['completed', 'failed', 'interrupted', 'cancelled', 'rolled_back']);
const preview = text => Array.from(text.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')).slice(0, 1000).join('');
const isUuid = value => typeof value === 'string' && value.length === 36
  && /^[0-9a-f]{8}-(?:[0-9a-f]{4}-){3}[0-9a-f]{12}$/i.test(value);

export async function socketRequest(path, value) {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection(path);
    socket.setEncoding('utf8');
    let response = '';
    socket.setTimeout(45000, () => socket.destroy(new Error('Local socket timed out.')));
    socket.on('connect', () => socket.write(`${typeof value === 'string' ? value : JSON.stringify(value)}\n`));
    socket.on('data', data => {
      response += data;
      if (response.includes('\n')) { socket.end(); resolve(response.trim()); }
    });
    socket.on('error', reject);
    socket.on('end', () => { if (!response) reject(new Error('Local socket closed without a response.')); });
  });
}

export class VoiceService {
  capture;
  speechWorker;
  recording = false;
  captureDevice = null;
  submitting = false;
  lastActivity = Date.now();
  state;
  config;
  queue = Promise.resolve();
  queued = 0;
  saving = Promise.resolve();
  saveTimer;
  deadlineTimer;
  spawnWorker = spawn;
  sentTimer;
  closing = false;
  memory;
  idleSeconds = Number(process.env.DOJO_VOICE_IDLE_SECONDS || 0);
  onIdle = async () => {};

  enqueue(operation) {
    if (this.queued >= 128 || this.closing) return Promise.reject(new Error('Voice queue is busy. Try again shortly.'));
    this.queued++;
    const result = this.queue.then(operation).finally(() => { this.queued--; });
    this.queue = result.catch(() => {});
    return result;
  }
  touch() { this.lastActivity = Date.now(); this.scheduleDeadline(); }
  scheduleDeadline(retryDelay = 0) {
    clearTimeout(this.deadlineTimer);
    this.deadlineTimer = undefined;
    if (this.closing || this.capture || this.submitting || this.active()?.activeRunId || this.active()?.awaitingRun) return;
    const delays = [];
    if (this.active()) delays.push(Math.max(0, 30_000 - (Date.now() - this.lastActivity)));
    if (this.idleSeconds > 0) delays.push(Math.max(0, this.idleSeconds * 1000 - (Date.now() - this.lastActivity)));
    if (!delays.length) return;
    this.deadlineTimer = setTimeout(() => {
      this.deadlineTimer = undefined;
      this.enqueue(async () => {
        await this.settle();
        return !this.capture && !this.submitting && !this.active()?.activeRunId && this.idleSeconds > 0 && Date.now() - this.lastActivity >= this.idleSeconds * 1000;
      }).then(async idle => { if (idle) await this.onIdle(); else if (!this.deadlineTimer) this.scheduleDeadline(); }).catch(error => { console.error(error.message); this.scheduleDeadline(1000); });
    }, Math.max(retryDelay, Math.min(...delays)));
  }
  deferSave() {
    if (this.saveTimer || this.closing) return;
    this.saveTimer = setTimeout(() => { this.saveTimer = undefined; this.save().catch(error => this.failure(error)); }, 250);
  }
  async shutdown() {
    this.closing = true;
    for (const timer of [this.deadlineTimer, this.saveTimer, this.sentTimer]) clearTimeout(timer);
    this.deadlineTimer = this.saveTimer = this.sentTimer = undefined;
    const canceled = this.cancel();
    this.speechWorker?.stdin.end();
    await canceled;
    await this.queue;
    if (this.state) await this.save();
    await this.runtime?.close();
    await this.memory?.close();
    for (const timer of [this.deadlineTimer, this.saveTimer, this.sentTimer]) clearTimeout(timer);
  }

  async start() {
    this.state = await readJson(statePath, { version: 1, activeThreadId: null, chats: [], seenMessages: [] });
    this.config = await readJson(join(dataDir, 'config.json'), {});
    this.memory = await MemoryStore.open(join(dataDir, 'memory-v2'));
    this.runtime = await PiRuntime.open({ stateDir: join(dataDir, 'pi'), root, memory: this.memory,
      config: { model: this.config.pi?.provider === 'anthropic' ? 'claude-haiku-5-5' : this.config.modelSelection?.model || this.state.memoryModelSelection?.model || 'gpt-6-luna', ...this.config.pi },
      onEvent: event => { if (!this.closing) this.enqueue(() => this.runtimeEvent(event)).catch(error => this.failure(error)); } });
    this.state.piConversationId = String(this.runtime.id);
    let chat = this.state.chats.find(item => item.threadId === this.state.piConversationId);
    if (!chat) { chat = { threadId: this.state.piConversationId, runtime: 'pi', createdAt: new Date().toISOString() }; this.state.chats.push(chat); }
    chat.activeRunId = this.runtime.busy() ? 'recovering' : null;
    this.state.activeThreadId = this.runtime.busy() ? this.state.piConversationId : null;
    this.runtime.resume();
    this.runtime.scheduleCompaction();
    this.ensureSpeechWorker();
  }
  active() { return this.state.chats.find(chat => chat.threadId === this.state.activeThreadId); }
  save() {
    clearTimeout(this.saveTimer); this.saveTimer = undefined;
    const snapshot = JSON.stringify(this.state);
    this.saving = this.saving.catch(() => {}).then(() => saveJson(statePath, null, snapshot));
    return this.saving;
  }
  async ui(event, value) {
    try { await socketRequest(join(runtimeDir, 'dojo.sock'), { event, value }); }
    catch (error) { console.error(`Dojo UI: ${error.message}`); }
  }
  async phase(value) { if (!this.capture || ['listening', 'transcribing'].includes(value)) await this.ui('phase', value); }
  async failure(error) {
    console.error(error.message);
    await this.ui('response', preview(`Could not complete that: ${error.message}`));
    await this.ui('phase', 'settled');
  }

  async runtimeEvent(event) {
    if (event.type === 'memory' && event.error) {
      console.error(`Dojo memory: ${event.error}`);
      this.state.memoryError = preview(event.error);
      await this.save();
    } else if (event.type === 'human_request') {
      this.pendingHumanRequest = event.request;
      await this.ui('response', preview(event.request.prompt));
    } else if (event.type === 'response') {
      const chat = this.active();
      if (this.capture && chat) chat.deferredResponse = event.text;
      else await this.ui('response', preview(event.text));
    } else if (event.type === 'run') {
      const chat = this.active();
      if (chat) {
        chat.activeRunId = this.runtime.busy() ? String(event.runId || 'preparing') : null;
        chat.status = event.status;
      }
      if (event.error) await this.failure(new Error(event.error));
      else await this.phase(this.runtime.busy() ? 'working' : 'settled');
      this.touch();
      await this.save();
    }
  }

  async submit(text) {
    if (!text.trim()) { await this.settle(true); await this.phase('settled'); return; }
    this.submitting = true;
    try {
      await this.ui('transcript', preview(text));
      let chat = this.state.chats.find(item => item.threadId === String(this.runtime.id));
      if (!chat) {
        chat = { threadId: String(this.runtime.id), runtime: 'pi', createdAt: new Date().toISOString() };
        this.state.chats.push(chat);
      }
      chat.title = text.slice(0, 60);
      chat.activeRunId = 'preparing';
      this.state.activeThreadId = String(this.runtime.id);
      await this.save();
      if (this.pendingHumanRequest?.status === 'pending' && this.pendingHumanRequest.expiresAt > Date.now()) {
        await this.runtime.respondHuman(this.pendingHumanRequest.id, text);
        this.pendingHumanRequest = undefined;
      }
      const receipt = await this.runtime.submit(text);
      await this.phase('sent');
      return receipt;
    } finally { this.submitting = false; this.touch(); }
  }

  async finish() {
    if (!this.capture || !this.recording) return;
    this.recording = false;
    if (process.env.NIRI_SOCKET) {
      try {
        const reply = JSON.parse(await socketRequest(process.env.NIRI_SOCKET, '"YieldHumanInput"'));
        if (reply.Err) console.error(`Desktop handoff: ${reply.Err}`);
      } catch (error) { console.error(`Desktop handoff: ${error.message}`); }
    }
    this.speechCommand('finish', this.capture.id);
  }
  async begin() {
    try { await readFile(heldPath); } catch (error) { if (error.code === 'ENOENT') return; throw error; }
    if (this.capture) return;
    await this.toggle();
  }
  async toggle() {
    if (this.capture) {
      if (this.recording) await this.finish();
      return;
    }
    this.ensureSpeechWorker();
    this.capture = { id: randomUUID() };
    this.recording = true;
    this.speechCommand('begin', this.capture.id);
    this.scheduleDeadline();
  }
  controllerBegin(owner) {
    if (!isUuid(owner)) throw new Error('Controller owner must be a UUID.');
    if (this.capture) {
      return { recording: this.recording, owned: this.capture.controllerOwner === owner, busy: this.capture.controllerOwner !== owner };
    }
    this.ensureSpeechWorker();
    this.capture = { id: randomUUID(), controllerOwner: owner };
    this.recording = true;
    this.speechCommand('begin', this.capture.id);
    this.scheduleDeadline();
    return { recording: true, owned: true, busy: false };
  }
  async controllerFinish(owner) {
    if (!isUuid(owner)) throw new Error('Controller owner must be a UUID.');
    if (this.capture?.controllerOwner !== owner) return { recording: this.recording, owned: false };
    await this.finish();
    return { recording: this.recording, owned: true };
  }
  async controllerCancel(owner) {
    if (!isUuid(owner)) throw new Error('Controller owner must be a UUID.');
    if (this.capture?.controllerOwner !== owner) return { recording: this.recording, owned: false };
    await this.cancel();
    return { recording: this.recording, owned: true };
  }
  speechCommand(action, id) {
    this.speechWorker.stdin.write(`${JSON.stringify({ action, id })}\n`);
  }
  ensureSpeechWorker() {
    if (this.speechWorker) return this.speechWorker;
    const nativeArgs = ['--model', this.config.transcriptionModel || 'parakeet_unified_en', 'serve',
      ...(this.config.microphone ? ['--device', this.config.microphone] : [])];
    const prepared = process.env.DOJO_VOICE_DEVENV === '1';
    const child = this.spawnWorker(prepared ? `${root}/target/debug/dojo-speech` : 'devenv',
      prepared ? nativeArgs : ['shell', '--', `${root}/target/debug/dojo-speech`, ...nativeArgs], {
      cwd: root, detached: true, stdio: ['pipe', 'pipe', 'pipe'],
    });
    this.speechWorker = child;
    let diagnostic = '';
    child.stderr.on('data', data => {
      diagnostic = (diagnostic + data).slice(-2000);
      console.error(data.toString().trim());
    });
    child.stdin.on('error', error => console.error(`Speech worker: ${error.message}`));
    const lines = createInterface({ input: child.stdout });
    lines.on('line', line => {
      let event;
      try { event = JSON.parse(line); } catch { return; }
      if (event.event === 'ready') { console.error(`${new Date().toISOString()} speech worker ready`); return; }
      if (event.id !== this.capture?.id) return;
      if (event.event === 'listening') { this.captureDevice = event.device; this.ui('phase', 'listening'); }
      if (event.event === 'transcribing') this.ui('phase', 'transcribing');
      if (['transcript', 'error'].includes(event.event)) {
        this.capture = null;
        this.recording = false;
        this.captureDevice = null;
        this.touch();
        this.enqueue(async () => {
          if (event.event === 'error') throw new Error(event.message);
          console.error(`Speech inference: ${event.inferenceMs} ms`);
          await this.submit(event.text || '');
        }).catch(error => this.failure(error));
      }
    });
    child.on('error', error => this.failure(error));
    child.on('close', code => {
      if (this.speechWorker !== child) return;
      this.speechWorker = null;
      if (this.capture) {
        this.capture = null;
        this.recording = false;
        this.captureDevice = null;
        this.failure(new Error(diagnostic.trim() || `Speech worker exited (${code}).`));
      }
    });
    return child;
  }

  async cancel() {
    if (this.capture) {
      const child = this.capture;
      this.capture = null;
      this.recording = false;
      this.captureDevice = null;
      this.speechCommand('cancel', child.id);
      const chat = this.active();
      if (chat?.deferredResponse) { await this.ui('response', preview(chat.deferredResponse)); delete chat.deferredResponse; }
      if (!this.closing) await this.settle();
      this.scheduleDeadline();
      await this.ui('phase', this.active()?.activeRunId ? 'working' : 'settled');
    }
  }
  async settle(force = false) {
    const chat = this.active();
    if (!chat || chat.activeRunId || chat.awaitingRun || chat.pending || this.capture || this.submitting) return;
    if (!force && Date.now() - this.lastActivity < 30_000) return;
    chat.settledAt = new Date().toISOString();
    this.state.activeThreadId = null;
    await this.save();
    this.scheduleDeadline();
  }
  async newChat() {
    await this.cancel();
    if (this.active()?.activeRunId) throw new Error('Stop the current agent before starting a new chat.');
    await this.settle(true);
    await this.phase('settled');
  }
  async handle(command) {
    console.error(`${new Date().toISOString()} voice command: ${command.action}`);
    if (!['status', 'history', 'memory-zoom', 'memory-date', 'memory-status'].includes(command.action)) this.touch();
    switch (command.action) {
      case 'memory-zoom': if (!this.memory) throw new Error('Dojo memory is disabled.'); return this.memory.zoom(command.id, command.count ?? 1, command.offset ?? 0);
      case 'memory-date': if (!this.memory) throw new Error('Dojo memory is disabled.'); return { date: await this.memory.date(command.id) };
      case 'memory-status': return { enabled: !!this.memory, runtime: 'pi', conversationId: this.runtime?.id || null,
        compacting: !!this.memory?.compacting, messages: this.memory?.records.length || 0 };
      case 'begin': await this.begin(); return { recording: this.recording };
      case 'finish': await this.finish(); return {};
      case 'toggle': await this.toggle(); return { recording: this.recording };
      case 'cancel': await this.cancel(); return {};
      case 'controller-begin': return this.controllerBegin(command.owner);
      case 'controller-finish': return this.controllerFinish(command.owner);
      case 'controller-cancel': return this.controllerCancel(command.owner);
      case 'send': await this.submit(command.text); return { threadId: this.state.activeThreadId };
      case 'new': await this.newChat(); return {};
      case 'history': return { activeThreadId: this.state.activeThreadId, chats: this.state.chats.map(({threadId,title,status}) => ({threadId,title,status})) };
      case 'resume': {
        if (String(command.threadId) !== String(this.runtime.id)) throw new Error('Earlier T3 conversations remain available in T3; Dojo now uses its continuous Pi conversation.');
        this.state.activeThreadId = String(this.runtime.id);
        return { threadId: String(this.runtime.id) };
      }
      case 'stop': await this.cancel(); await this.runtime.stop(); return {};
      case 'status': return { recording: this.recording, transcribing: !!this.capture && !this.recording,
        microphone: this.captureDevice,
        threadId: this.state.activeThreadId, runId: this.active()?.activeRunId || null };
      default: throw new Error('Unknown voice command.');
    }
  }
}

export async function acquireServerLock(path = join(runtimeDir, 'dojo-voice.lock')) {
  const child = spawn('flock', ['--no-fork', '-n', '-E', '73', path, 'sh', '-c', 'printf "locked\\n"; exec cat >/dev/null'], { stdio: ['pipe', 'pipe', 'inherit'] });
  child.stdin.on('error', () => {});
  const acquired = await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.stdout.once('data', () => resolve(true));
    child.once('exit', code => code === 73 ? resolve(false) : reject(new Error(`Voice lock holder exited (${code}).`)));
  });
  if (!acquired) return null;
  child.stdout.destroy();
  return child;
}

export async function serve(service = new VoiceService()) {
  const lock = await acquireServerLock();
  if (!lock) return;
  let server;
  let socketIdentity;
  const connections = new Set();
  let stopped = false;
  let shutdownPromise;
  const shutdown = () => {
    if (shutdownPromise) return shutdownPromise;
    stopped = true;
    server?.close();
    for (const socket of connections) socket.destroy();
    shutdownPromise = (async () => {
      try { await service.shutdown(); }
      finally {
        const current = await stat(socketPath).catch(() => null);
        if (socketIdentity && current?.ino === socketIdentity.ino && current?.dev === socketIdentity.dev) await unlink(socketPath).catch(() => {});
        lock.stdin.end();
        if (lock.exitCode == null && lock.signalCode == null) await new Promise(resolve => lock.once('exit', resolve));
      }
    })();
    return shutdownPromise;
  };
  service.onIdle = shutdown;
  lock.once('exit', () => { if (!stopped) shutdown().catch(error => console.error(error.message)); });
  try {
    await service.start();
    await unlink(socketPath).catch(error => { if (error.code !== 'ENOENT') throw error; });
    server = net.createServer(socket => {
      if (connections.size >= 32 || stopped) { socket.destroy(); return; }
      connections.add(socket);
      socket.setEncoding('utf8');
      socket.once('close', () => connections.delete(socket));
      socket.on('error', () => socket.destroy());
      let buffer = '';
      socket.setTimeout(45000, () => socket.destroy());
      socket.on('data', data => {
        buffer += data;
        if (buffer.length > maxCommandCodeUnits) return socket.destroy();
        if (!buffer.includes('\n')) return;
        socket.pause();
        let command;
        try { command = JSON.parse(buffer.slice(0, buffer.indexOf('\n'))); }
        catch { socket.end('{"error":"Invalid command"}\n'); return; }
        const operation = ['begin', 'finish', 'toggle', 'cancel', 'controller-begin', 'controller-finish', 'controller-cancel', 'status', 'history'].includes(command.action)
          ? service.handle(command)
          : service.enqueue(() => service.handle(command));
        operation.then(result => socket.end(`${JSON.stringify(result)}\n`), async error => {
          await service.failure(error);
          socket.end(`${JSON.stringify({ error: error.message })}\n`);
        });
      });
    });
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(socketPath, resolve); });
    socketIdentity = await stat(socketPath);
    await chmod(socketPath, 0o600);
    service.scheduleDeadline();
    for (const signal of ['SIGTERM', 'SIGINT']) process.once(signal, () => {
      shutdown().then(() => process.exit(0), error => { console.error(error.message); process.exit(1); });
    });
    return { service, server, lock, shutdown };
  } catch (error) { await shutdown(); throw error; }
}

async function request(command) {
  try { return JSON.parse(await socketRequest(socketPath, command)); }
  catch (error) {
    if (!['ENOENT', 'ECONNREFUSED'].includes(error.code)) throw error;
    if (['cancel', 'finish'].includes(command.action)) return {};
    await mkdir(dataDir, { recursive: true, mode: 0o700 });
    const log = await open(join(dataDir, 'voice.log'), 'a', 0o600);
    const child = spawn(process.execPath, [fileURLToPath(import.meta.url), '--serve'], {
      detached: true, stdio: ['ignore', log.fd, log.fd], env: process.env,
    });
    child.unref(); await log.close();
    for (let attempt = 0; attempt < 50; attempt++) {
      await new Promise(resolve => setTimeout(resolve, 100));
      try { return JSON.parse(await socketRequest(socketPath, command)); }
      catch (error) { if (!['ENOENT', 'ECONNREFUSED'].includes(error.code)) throw error; }
    }
    throw new Error('Voice worker did not start. Check ~/.local/state/dojo/voice/voice.log.');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  if (process.argv[2] === '--serve') await serve();
  else {
    const action = process.argv[2] || 'begin';
    const command = { action, ...(action === 'send' ? { text: process.argv.slice(3).join(' ') } : {}),
      ...(action === 'resume' ? { threadId: process.argv[3] } : {}) };
    if (action === 'begin') await writeFile(heldPath, '', { mode: 0o600 });
    if (['finish', 'cancel'].includes(action)) await unlink(heldPath).catch(error => { if (error.code !== 'ENOENT') throw error; });
    const result = await request(command);
    console.log(JSON.stringify(result));
    if (result.error) process.exitCode = 1;
  }

}
