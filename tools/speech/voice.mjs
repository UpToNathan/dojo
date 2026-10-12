import net from 'node:net';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, unlink, chmod, open, stat } from 'node:fs/promises';
import { createInterface } from 'node:readline';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { randomUUID } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { T3Client, dataDir, readJson, saveJson } from './t3-client.mjs';

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const runtimeDir = process.env.XDG_RUNTIME_DIR;
if (!runtimeDir) throw new Error('XDG_RUNTIME_DIR is required.');
const socketPath = join(runtimeDir, 'dojo-voice.sock');
const heldPath = join(runtimeDir, 'dojo-voice-held');
const statePath = join(dataDir, 'conversations.json');
const maxCommandCodeUnits = 16_384;
const terminal = new Set(['completed', 'failed', 'interrupted', 'cancelled', 'rolled_back']);
const preview = text => Array.from(text.replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f]/g, '')).slice(0, 1000).join('');

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
  client;
  subscription;
  capture;
  speechWorker;
  recording = false;
  captureDevice = null;
  submitting = false;
  reconnecting = false;
  lastActivity = Date.now();
  state;
  config;
  queue = Promise.resolve();
  queued = 0;
  saving = Promise.resolve();
  saveTimer;
  deadlineTimer;
  reconnectTimer;
  reconnectAttempts = 0;
  spawnWorker = spawn;
  sentTimer;
  connecting;
  closing = false;
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
    if (this.closing || this.capture || this.submitting || this.active()?.activeRunId || this.reconnecting) return;
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
    for (const timer of [this.deadlineTimer, this.saveTimer, this.reconnectTimer, this.sentTimer]) clearTimeout(timer);
    this.deadlineTimer = this.saveTimer = this.reconnectTimer = this.sentTimer = undefined;
    const canceled = this.cancel();
    this.speechWorker?.stdin.end();
    this.client?.close();
    await canceled;
    await this.queue;
    if (this.state) await this.save();
    for (const timer of [this.deadlineTimer, this.saveTimer, this.reconnectTimer, this.sentTimer]) clearTimeout(timer);
  }

  async start() {
    this.state = await readJson(statePath, { version: 1, activeThreadId: null, chats: [], seenMessages: [] });
    this.config = await readJson(join(dataDir, 'config.json'), {});
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

  async connect() {
    if (this.closing) throw new Error('Voice service is shutting down.');
    if (this.client?.socket?.readyState === WebSocket.OPEN) return this.client;
    if (this.connecting) return this.connecting;
    this.connecting = (async () => {
      const client = new T3Client();
      await client.connect();
      if (this.closing) { client.close(); throw new Error('Voice service is shutting down.'); }
      this.client = client;
      client.onDisconnect = () => {
        if (this.client !== client) return;
        this.subscription = null;
        this.scheduleReconnect();
      };
      return client;
    })();
    try { return await this.connecting; }
    finally { this.connecting = undefined; }
  }
  scheduleReconnect() {
    if (this.closing || !this.active() || (!this.active().activeRunId && !this.active().pending && !this.submitting) || this.reconnectTimer) return;
    this.reconnecting = true;
    this.scheduleDeadline();
    this.reconnectTimer = setTimeout(async () => {
      this.reconnectTimer = undefined;
      try { await this.watch(); this.reconnectAttempts = 0; }
      catch (error) { console.error(`T3 reconnect: ${error.message}`); this.scheduleReconnect(); }
      finally { this.reconnecting = !!this.reconnectTimer; this.scheduleDeadline(); }
    }, Math.min(30_000, 1000 * 2 ** Math.min(this.reconnectAttempts++, 5)));
  }

  async watch(replay = true) {
    const chat = this.active();
    if (!chat) return;
    const client = await this.connect();
    if (this.subscription?.client === client && this.subscription.threadId === chat.threadId) return;
    if (this.subscription) this.subscription.client.unsubscribe(this.subscription.id);
    this.subscription = client.subscribe(chat.threadId, item => {
      return this.enqueue(() => this.stream(chat, item)).catch(error => { client.close(); throw error; });
    }, replay ? chat.sequence : undefined);
    this.subscription.client = client;
    this.subscription.threadId = chat.threadId;
    const subscription = this.subscription;
    this.subscription.promise.then(() => {
      if (this.subscription === subscription) { this.subscription = null; this.scheduleReconnect(); }
    }, error => {
      if (this.subscription === subscription) {
        this.subscription = null;
        if (replay && chat.sequence != null && this.client === client && client.socket.readyState === WebSocket.OPEN && !this.closing) this.watch(false).catch(error => this.failure(error));
        else this.scheduleReconnect();
      }
      if (error.message !== 'T3 disconnected.') console.error(`T3 subscription: ${error.message}`);
    });
  }

  async stream(chat, item) {
    if (chat.threadId !== this.state.activeThreadId) return;
    if (item.kind === 'snapshot') {
      for (const message of item.projection.messages) await this.message(chat, message);
      const latest = item.projection.runs.at(-1);
      if (latest) await this.run(chat, latest);
      chat.sequence = item.snapshotSequence;
    } else if (item.kind === 'event') {
      if (item.sequence <= (chat.sequence ?? -1)) return;
      const { type, payload } = item.event;
      if (type === 'message.updated') await this.message(chat, payload);
      if (type === 'run.updated') await this.run(chat, payload);
      if (type === 'runtime-request.updated' && payload.status === 'pending') {
        await this.ui('response', 'The agent needs your input. Open this chat in T3 to answer its request.');
      }
      chat.sequence = item.sequence;
    }
    if (item.kind === 'event' && (item.event.type !== 'run.updated') && !(item.event.type === 'message.updated' && !item.event.payload.streaming)) this.deferSave();
    else await this.save();
    this.scheduleDeadline();
  }

  async message(chat, message) {
    if (message.role !== 'assistant' || message.streaming || !message.text.trim()) return;
    if (this.state.seenMessages.includes(message.id)) return;
    if (message.createdAt < chat.listenSince) return;
    this.state.seenMessages.push(message.id);
    this.state.seenMessages = this.state.seenMessages.slice(-128);
    if (this.capture) chat.deferredResponse = message.text;
    else await this.ui('response', preview(message.text));
  }
  async run(chat, run) {
    if (run.requestedAt < chat.listenSince) return;
    chat.activeRunId = terminal.has(run.status) ? null : run.id;
    chat.status = run.status;
    this.touch();
    if (terminal.has(run.status)) {
      if (this.capture || this.submitting) return;
      await this.settle();
      await this.phase('settled');
      if (run.status === 'failed') await this.ui('response', 'The agent run failed. Its details are available in T3.');
    } else if (run.status === 'running' || run.status === 'starting') await this.phase('working');
  }

  async submit(text) {
    text = text.trim();
    if (!text) { await this.settle(); await this.phase('settled'); return; }
    await this.ui('transcript', preview(text));
    this.submitting = true;
    try {
      const client = await this.connect();
      let chat = this.active();
      if (!chat) {
        const config = await client.call('server.getConfig');
        const selected = this.config.modelSelection || config.settings.defaultModelSelection;
        const modelSelection = selected?.instanceId === 'codex'
          ? { ...selected, options: [
              ...(Array.isArray(selected.options) ? selected.options : []).filter(option => option.id !== 'reasoningEffort'),
              { id: 'reasoningEffort', value: 'none' },
            ] } : selected;
        if (!modelSelection) throw new Error('Choose a default agent model in T3.');
        const { projectId } = await client.call('projects.ensureScratch');
        chat = { threadId: randomUUID(), projectId, title: text.slice(0, 60), modelSelection,
          createdAt: new Date().toISOString(), listenSince: new Date().toISOString(), activeRunId: null };
        this.state.chats.push(chat);
        this.state.activeThreadId = chat.threadId;
        await this.save();
      }
      if (!chat.launched) {
        let result;
        try { result = { projection: await client.call('orchestration.getThreadProjection', { threadId: chat.threadId }) }; }
        catch { result = await client.call('orchestration.launchThread', {
          commandId: chat.threadId, threadId: chat.threadId,
          projectId: chat.projectId, title: chat.title, modelSelection: chat.modelSelection,
          runtimeMode: this.config.runtimeMode || 'full-access', interactionMode: 'default',
          workspaceStrategy: { type: 'root' },
        }); }
        chat.workspacePath = result.projection.thread.worktreePath;
        chat.launched = true;
        await this.save();
      }
      if (chat.workspacePath) {
        const instructions = await readFile(join(root, 'tools/speech/assistant.md'), 'utf8');
        await writeFile(join(chat.workspacePath, 'AGENTS.md'), instructions);
        const mcp = { command: join(root, 'target/debug/dojo-agent'), args: ['--name', 'dojo-voice'] };
        await writeFile(join(chat.workspacePath, '.mcp.json'), JSON.stringify({ mcpServers: { 'dojo-desktop': mcp } }), { flag: 'wx' }).catch(error => { if (error.code !== 'EEXIST') throw error; });
        await writeFile(join(chat.workspacePath, 'opencode.json'), JSON.stringify({ mcp: { 'dojo-desktop': {
          type: 'local', command: [mcp.command, ...mcp.args], enabled: true,
        } } }), { flag: 'wx' }).catch(error => { if (error.code !== 'EEXIST') throw error; });
      }
      chat.listenSince = new Date().toISOString();
      chat.pending = { type: 'message.dispatch', commandId: randomUUID(), threadId: chat.threadId,
        messageId: randomUUID(), text, attachments: [], deliveryIntent: 'auto',
        createdBy: 'user', creationSource: 'web',
        dispatchMode: { type: 'start_immediately' } };
      await this.save();
      await this.watch();
      await client.call('orchestration.dispatchCommand', chat.pending);
      delete chat.pending;
      await this.save();
      await this.phase('sent');
      clearTimeout(this.sentTimer);
      this.sentTimer = setTimeout(() => { this.sentTimer = undefined; if (this.active()?.activeRunId) this.phase('working'); }, 500);
    } finally {
      this.submitting = false; this.touch();
      const chat = this.active();
      if (chat?.deferredResponse) {
        await this.ui('response', preview(chat.deferredResponse));
        delete chat.deferredResponse;
        await this.save();
      }
    }
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
    if (!chat || chat.activeRunId || this.capture || this.submitting) return;
    if (!force && Date.now() - this.lastActivity < 30_000) return;
    await (await this.connect()).call('orchestration.dispatchCommand', {
      type: 'thread.settle', commandId: chat.settleCommandId ||= randomUUID(), threadId: chat.threadId,
    });
    chat.settledAt = new Date().toISOString();
    if (this.subscription) this.client?.unsubscribe(this.subscription.id);
    this.subscription = null;
    clearTimeout(this.reconnectTimer); this.reconnectTimer = undefined; this.reconnecting = false;
    this.state.activeThreadId = null;
    await this.save();
    this.client?.close(); this.client = undefined;
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
    if (!['status', 'history'].includes(command.action)) this.touch();
    switch (command.action) {
      case 'begin': await this.begin(); return { recording: this.recording };
      case 'finish': await this.finish(); return {};
      case 'toggle': await this.toggle(); return { recording: this.recording };
      case 'cancel': await this.cancel(); return {};
      case 'send': await this.submit(command.text); return { threadId: this.state.activeThreadId };
      case 'new': await this.newChat(); return {};
      case 'history': return { activeThreadId: this.state.activeThreadId, chats: this.state.chats.map(({threadId,title,status}) => ({threadId,title,status})) };
      case 'resume': {
        await this.newChat();
        const chat = this.state.chats.find(chat => chat.threadId === command.threadId);
        if (!chat) throw new Error('This is not a Dojo conversation.');
        this.state.activeThreadId = chat.threadId;
        chat.listenSince = new Date().toISOString();
        await this.save(); await this.watch(); return { threadId: chat.threadId };
      }
      case 'stop': {
        await this.cancel();
        const chat = this.active();
        if (chat?.activeRunId) await (await this.connect()).call('orchestration.dispatchCommand', {
          type: 'run.interrupt', commandId: randomUUID(), threadId: chat.threadId,
          runId: chat.activeRunId, holdQueue: true,
        });
        return {};
      }
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
        const operation = ['begin', 'finish', 'toggle', 'cancel', 'status', 'history'].includes(command.action)
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
    if (service.active()) {
      if (service.active().activeRunId) await service.watch().catch(error => service.failure(error));
      else await service.settle().catch(error => service.failure(error));
    }
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
