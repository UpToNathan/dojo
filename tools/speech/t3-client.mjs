import { readFile, mkdir, writeFile, rename } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { randomUUID } from 'node:crypto';

const execute = promisify(execFile);
export const dataDir = join(process.env.XDG_STATE_HOME || join(homedir(), '.local/state'), 'dojo/voice');
export async function saveJson(path, value, serialized = JSON.stringify(value)) {
  await mkdir(dataDir, { recursive: true, mode: 0o700 });
  const temporary = `${path}.${randomUUID()}.tmp`;
  await writeFile(temporary, serialized, { mode: 0o600 });
  await rename(temporary, path);
}
export async function readJson(path, fallback) {
  try { return JSON.parse(await readFile(path, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return fallback; throw error; }
}

export class T3Client {
  pending = new Map();
  nextId = 0;
  socket;
  onDisconnect = () => {};

  async connect() {
    const baseDir = process.env.T3CODE_HOME || join(homedir(), '.t3');
    const runtime = await readJson(join(baseDir, 'userdata/server-runtime.json'));
    if (!runtime?.port) throw new Error('T3 backend is not running.');
    const origin = `http://127.0.0.1:${runtime.port}`;
    const credentialPath = join(dataDir, 't3-session.json');
    let credential = await readJson(credentialPath, null);
    if (!credential || credential.baseDir !== baseDir || credential.expiresAt < Date.now() + 60_000) {
      const { stdout } = await execute('t3', ['auth', 'session', 'issue', '--base-dir', baseDir,
        '--scope', 'orchestration:read', '--scope', 'orchestration:operate',
        '--ttl', '1d', '--label', 'Dojo voice', '--token-only'], { timeout: 30_000 });
      credential = { token: stdout.trim(), expiresAt: Date.now() + 23 * 3600_000, baseDir };
      if (!credential.token || /\s/.test(credential.token)) throw new Error('T3 did not issue a client credential.');
      await saveJson(credentialPath, credential);
    }
    const response = await fetch(`${origin}/api/auth/websocket-ticket`, {
      method: 'POST', headers: { authorization: `Bearer ${credential.token}` },
      signal: AbortSignal.timeout(10_000),
    });
    if (!response.ok) throw new Error(`T3 authentication failed (${response.status}).`);
    const { ticket } = await response.json();
    const url = new URL('/ws', origin.replace('http:', 'ws:'));
    url.searchParams.set('orchestrationProtocol', '2');
    url.searchParams.set('wsTicket', ticket);
    this.socket = new WebSocket(url);
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => { this.socket.close(); reject(new Error('T3 connection timed out.')); }, 10_000);
      this.socket.addEventListener('open', () => { clearTimeout(timeout); resolve(); }, { once: true });
      this.socket.addEventListener('error', () => { clearTimeout(timeout); reject(new Error('T3 WebSocket connection failed.')); }, { once: true });
    });
    this.socket.addEventListener('message', ({ data }) => {
      try {
        const frames = JSON.parse(data);
        for (const frame of Array.isArray(frames) ? frames : [frames]) this.receive(frame).catch(error => { console.error(error.message); this.close(); });
      } catch (error) { console.error(`Invalid T3 frame: ${error.message}`); this.close(); }
    });
    this.socket.addEventListener('close', () => {
      for (const pending of this.pending.values()) pending.reject(new Error('T3 disconnected.'));
      this.pending.clear();
      this.onDisconnect();
    });
  }

  async receive(frame) {
    if (frame._tag === 'Ping') return this.socket.send(JSON.stringify({ _tag: 'Pong' }));
    const pending = this.pending.get(frame.requestId);
    if (!pending) return;
    if (frame._tag === 'Chunk') {
      for (const value of frame.values) await pending.onValue?.(value);
      this.socket.send(JSON.stringify({ _tag: 'Ack', requestId: frame.requestId }));
    } else if (frame._tag === 'Exit') {
      this.pending.delete(frame.requestId);
      if (frame.exit._tag === 'Success') pending.resolve(frame.exit.value);
      else pending.reject(new Error(JSON.stringify(frame.exit.cause)));
    }
  }

  request(tag, payload, onValue) {
    const id = String(++this.nextId);
    let timeout;
    const promise = new Promise((resolve, reject) => {
      if (this.pending.size >= 128) { reject(new Error('Too many pending T3 requests.')); return; }
      this.pending.set(id, { resolve, reject, onValue });
      try { this.socket.send(JSON.stringify({ _tag: 'Request', id, tag, payload, headers: [] })); }
      catch (error) { this.pending.delete(id); reject(error); return; }
      if (!onValue) timeout = setTimeout(() => {
        this.pending.delete(id);
        if (this.socket.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify({ _tag: 'Interrupt', requestId: id }));
        reject(new Error(`T3 ${tag} timed out.`));
      }, 30_000);
    }).finally(() => clearTimeout(timeout));
    return { id, promise };
  }
  call(tag, payload = {}) { return this.request(tag, payload).promise; }
  subscribe(threadId, onValue, afterSequence) {
    const request = this.request('orchestration.subscribeThread', {
      threadId, ...(afterSequence == null ? {} : { afterSequence }), requestCompletionMarker: true,
    }, onValue);
    return request;
  }
  unsubscribe(id) {
    const pending = this.pending.get(id);
    if (!pending) return;
    this.pending.delete(id);
    pending.resolve();
    if (this.socket.readyState === WebSocket.OPEN) this.socket.send(JSON.stringify({ _tag: 'Interrupt', requestId: id }));
  }
  close() { this.socket?.close(); }
}
