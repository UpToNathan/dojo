import { promises as fs } from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';

const bytes = text => Buffer.byteLength(text, 'utf8');
const keyOf = (l, i) => `${l}:${i}`;
const range = ([l, i]) => ({ id: i * 2 ** l, count: 2 ** l });
const kinds = new Set(['user', 'unii', 'tool', 'echo', 'work', 'note']);
const MAX_TEXT = 30_000;
function prefix(text, limit) {
  const buffer = Buffer.from(text);
  let end = Math.min(limit, buffer.length);
  while (end < buffer.length && (buffer[end] & 0xc0) === 0x80) end--;
  return buffer.subarray(0, end).toString('utf8');
}
async function syncDirectory(directory) {
  const handle = await fs.open(directory, 'r');
  try { await handle.sync(); } finally { await handle.close(); }
}
async function atomic(file, value) {
  const temporary = `${file}.tmp`;
  const handle = await fs.open(temporary, 'w', 0o600);
  try { await handle.writeFile(JSON.stringify(value)); await handle.sync(); }
  finally { await handle.close(); }
  await fs.rename(temporary, file);
  await syncDirectory(path.dirname(file));
}
async function lock(file) {
  const handle = await fs.open(file, 'a', 0o600);
  await handle.close();
  const child = spawn('flock', ['--no-fork', '-w', '1', '-E', '73', file, 'sh', '-c',
    'printf "locked\\n"; exec cat >/dev/null'], { stdio: ['pipe', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', code => reject(new Error(`Memory directory already owned or lock failed (${code})`)));
    child.stdout.once('data', resolve);
  });
  return child;
}
function validateMessage(message) {
  if (!Number.isSafeInteger(message.i) || message.i < 0 || !kinds.has(message.kind) ||
      typeof message.text !== 'string' || message.size !== bytes(message.text) ||
      typeof message.date !== 'string' || !Number.isFinite(Date.parse(message.date)))
    throw new Error('Invalid memory message');
}
function validateNode(node) {
  if (!Number.isSafeInteger(node.l) || node.l < 0 || node.l > 52 ||
      !Number.isSafeInteger(node.i) || node.i < 0 || !Number.isSafeInteger(node.i * 2 ** node.l) ||
      typeof node.text !== 'string' || !node.text.trim() || node.size !== bytes(node.text))
    throw new Error('Invalid memory node');
}

/** One process owns the archive. Model workers return text and never write files. */
export class MemoryStore {
  static async open(directory, { nodeBytes = 512, viewMaxBytes = 128_000, viewMinBytes = 64_000,
    compactionMaxBytes = 32_000, compactionMinBytes = 16_000 } = {}) {
    if (![nodeBytes, viewMaxBytes, viewMinBytes, compactionMaxBytes, compactionMinBytes].every(Number.isSafeInteger) ||
        nodeBytes !== 512 || viewMinBytes < 32 || viewMaxBytes <= viewMinBytes ||
        compactionMinBytes < 32 || compactionMaxBytes <= compactionMinBytes)
      throw new Error('Invalid memory byte budgets; nodes use 512 bytes');
    if (!path.isAbsolute(directory) || path.resolve(directory) !== directory || directory === path.parse(directory).root)
      throw new Error('Memory directory must be a normalized owned absolute path');
    const store = new MemoryStore();
    Object.assign(store, { directory, nodeBytes, viewMaxBytes, viewMinBytes, compactionMaxBytes,
      compactionMinBytes, records: [], locations: new Map(), keys: new Map(), nodes: new Map(),
      refs: [], compactionRefs: [], frontier: 0, shrinking: false, compactionShrinking: false,
      queue: [], queued: new Set(), failed: new Map(), waitingLeaves: [], leafWindow: new Set(),
      activeJobs: new Set(), appendEpoch: 0, chain: Promise.resolve(), closed: false, treeCount: 0 });
    await fs.mkdir(directory, { recursive: true, mode: 0o700 });
    const entry = await fs.lstat(directory);
    if (!entry.isDirectory() || entry.isSymbolicLink() || (process.getuid && entry.uid !== process.getuid()))
      throw new Error('Memory directory must be owned and cannot be a symlink');
    await fs.chmod(directory, 0o700);
    store.owner = await lock(path.join(directory, 'owner.lock'));
    try {
      for (const name of ['main', 'tree', 'state']) {
        const child = path.join(directory, name);
        await fs.mkdir(child, { recursive: true, mode: 0o700 });
        const entry = await fs.lstat(child);
        if (!entry.isDirectory() || entry.isSymbolicLink() || (process.getuid && entry.uid !== process.getuid()))
          throw new Error('Memory child directory must be owned and cannot be a symlink');
        await fs.chmod(child, 0o700);
      }
      await store.load();
      store.recoverQueues();
      return store;
    } catch (error) { await store.close(); throw error; }
  }
  stateFile(name) { return path.join(this.directory, 'state', name); }
  serialize(action) {
    const result = this.chain.then(() => {
      if (this.closed) throw new Error('Memory store is closed');
      if (this.writeFault) throw new Error('Memory write failed; reopen the store to recover.', { cause: this.writeFault });
      if (this.owner.exitCode !== null || this.owner.signalCode !== null)
        throw new Error('Memory ownership was lost; restart Dojo voice.');
      return action();
    });
    this.chain = result.catch(() => {});
    return result;
  }
  async readLines(file) {
    let buffer;
    try { buffer = await fs.readFile(file); } catch (error) { if (error.code === 'ENOENT') return []; throw error; }
    const end = buffer.lastIndexOf(10) + 1;
    const entries = [];
    let offset = 0;
    for (const line of buffer.subarray(0, end).toString('utf8').split('\n').slice(0, -1)) {
      const length = bytes(line) + 1;
      entries.push({ value: JSON.parse(line), file, offset, length });
      offset += length;
    }
    // Parse complete lines before repairing only an incomplete final append.
    if (end !== buffer.length) {
      const handle = await fs.open(file, 'r+');
      try { await handle.truncate(end); await handle.sync(); } finally { await handle.close(); }
    }
    return entries;
  }
  async loadDaily(category) {
    const names = (await fs.readdir(path.join(this.directory, category))).filter(name => /^\d{4}-\d{2}-\d{2}\.jsonl$/.test(name)).sort();
    const entries = [];
    for (const name of names) entries.push(...await this.readLines(path.join(this.directory, category, name)));
    return entries;
  }
  rememberMessage(message, location) {
    validateMessage(message);
    if (message.i !== this.records.length) throw new Error('Noncontiguous permanent message IDs');
    const { text, ...metadata } = message;
    this.records.push({ ...metadata, id: message.i });
    this.locations.set(message.i, location);
  }
  rememberReceipt(receipt) {
    if (typeof receipt.key !== 'string' || !receipt.key || !Number.isSafeInteger(receipt.id) ||
        !Number.isSafeInteger(receipt.count) || receipt.count < 1 || receipt.id < 0 ||
        receipt.id + receipt.count > this.records.length || typeof receipt.threadId !== 'string' ||
        typeof receipt.complete !== 'boolean')
      throw new Error('Invalid source receipt');
    const old = this.keys.get(receipt.key);
    if (old && JSON.stringify(old) !== JSON.stringify(receipt)) throw new Error('Conflicting source receipt');
    this.keys.set(receipt.key, receipt);
    for (let i = receipt.id; i < receipt.id + receipt.count; i++) {
      if (this.records[i].key && this.records[i].key !== receipt.key) throw new Error('Overlapping source receipts');
      Object.assign(this.records[i], { key: receipt.key, threadId: receipt.threadId, complete: receipt.complete });
    }
  }
  checkpoint() {
    return { version: 2, length: this.records.length, treeCount: this.treeCount,
      refs: this.refs, compactionRefs: this.compactionRefs, frontier: this.frontier,
      shrinking: this.shrinking, compactionShrinking: this.compactionShrinking,
      appendEpoch: this.appendEpoch };
  }
  restore(checkpoint) {
    if (checkpoint.version !== 2 || !Number.isSafeInteger(checkpoint.length) || checkpoint.length < 0 ||
        checkpoint.length > this.records.length || !Number.isSafeInteger(checkpoint.treeCount) ||
        checkpoint.treeCount < 0 || checkpoint.treeCount > this.treeCount ||
        !Number.isSafeInteger(checkpoint.appendEpoch) || checkpoint.appendEpoch < 0 || !Number.isSafeInteger(checkpoint.frontier) ||
        checkpoint.frontier < 0 || checkpoint.frontier > checkpoint.length ||
        typeof checkpoint.shrinking !== 'boolean' || typeof checkpoint.compactionShrinking !== 'boolean')
      throw new Error('Invalid memory checkpoint');
    this.checkRefs(checkpoint.refs, checkpoint.length);
    this.checkRefs(checkpoint.compactionRefs, checkpoint.frontier, true);
    Object.assign(this, { refs: checkpoint.refs, compactionRefs: checkpoint.compactionRefs,
      frontier: checkpoint.frontier, shrinking: checkpoint.shrinking,
      compactionShrinking: checkpoint.compactionShrinking, appendEpoch: checkpoint.appendEpoch });
  }
  checkRefs(refs, length, built = false) {
    if (!Array.isArray(refs)) throw new Error('Invalid persisted view');
    let cursor = 0;
    for (const ref of refs) {
      if (!Array.isArray(ref) || ref.length !== 2) throw new Error('Invalid persisted node ref');
      const [l, i] = ref;
      if (!Number.isSafeInteger(l) || l < 0 || l > 52 || !Number.isSafeInteger(i) || i < 0)
        throw new Error('Invalid persisted node ref');
      const { id, count } = range(ref);
      if (id !== cursor || !Number.isSafeInteger(id + count) || id + count > length ||
          ((built || l > 0) && !this.nodes.has(keyOf(l, i)))) throw new Error('Invalid persisted view coverage');
      cursor += count;
    }
    if (cursor !== length) throw new Error('Invalid persisted view coverage');
  }
  async load() {
    const messages = await this.loadDaily('main');
    messages.sort((a, b) => a.value.i - b.value.i);
    for (const entry of messages) this.rememberMessage(entry.value, entry);
    for (const { value } of await this.loadDaily('tree')) {
      validateNode(value);
      const { id, count } = range([value.l, value.i]);
      if (id + count > this.records.length || this.nodes.has(keyOf(value.l, value.i)))
        throw new Error('Invalid or duplicate persisted node');
      this.nodes.set(keyOf(value.l, value.i), value); this.treeCount++;
    }
    for (const node of this.nodes.values()) {
      if (node.l > 0 && (!this.nodes.has(keyOf(node.l - 1, node.i * 2)) ||
          !this.nodes.has(keyOf(node.l - 1, node.i * 2 + 1)))) throw new Error('Persisted node is missing its children');
    }
    for (const { value } of await this.readLines(this.stateFile('sources.jsonl'))) this.rememberReceipt(value);
    let pending, checkpoint;
    try { pending = JSON.parse(await fs.readFile(this.stateFile('pending.json'), 'utf8')); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    try { checkpoint = JSON.parse(await fs.readFile(this.stateFile('checkpoint.json'), 'utf8')); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    if (pending) {
      await this.applyTransaction(pending);
      checkpoint = pending.checkpoint;
    }
    if (!checkpoint) {
      if (this.records.length || this.nodes.size) throw new Error('Missing persisted view checkpoint');
      await this.saveCheckpoint(this.checkpoint());
    } else {
      this.restore(checkpoint);
      if (checkpoint.length !== this.records.length || checkpoint.treeCount !== this.treeCount)
        throw new Error('Archive extends beyond its committed view without a recovery transaction');
      // A completed checkpoint owns these exact arrays. Repair interrupted atomic file replacements.
      for (const [name, refs] of [['view.json', this.refs], ['compaction-view.json', this.compactionRefs]]) {
        let saved;
        try { saved = JSON.parse(await fs.readFile(path.join(this.directory, name), 'utf8')); }
        catch (error) { if (error.code !== 'ENOENT') throw error; }
        if (JSON.stringify(saved) !== JSON.stringify(refs)) await atomic(path.join(this.directory, name), refs);
      }
    }
    for (const record of this.records) if (!record.key) throw new Error('Message has no source receipt');
  }
  async appendLine(file, value) {
    const handle = await fs.open(file, 'a+', 0o600);
    let offset;
    const text = `${JSON.stringify(value)}\n`;
    try { offset = (await handle.stat()).size; await handle.writeFile(text); await handle.sync(); }
    finally { await handle.close(); }
    await syncDirectory(path.dirname(file));
    return { file, offset, length: bytes(text) };
  }
  async saveCheckpoint(checkpoint) {
    await atomic(path.join(this.directory, 'view.json'), checkpoint.refs);
    await atomic(path.join(this.directory, 'compaction-view.json'), checkpoint.compactionRefs);
    await atomic(this.stateFile('checkpoint.json'), checkpoint);
  }
  async applyTransaction(transaction) {
    if (transaction.version !== 2 || !Array.isArray(transaction.messages) || !Array.isArray(transaction.nodes) ||
        typeof transaction.day !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(transaction.day))
      throw new Error('Invalid recovery transaction');
    for (const message of transaction.messages) {
      if (message.i < this.records.length) {
        const existing = await this.readMessage(message.i);
        if (JSON.stringify(existing) !== JSON.stringify(message)) throw new Error('Recovery message differs');
      } else {
        const file = path.join(this.directory, 'main', `${message.date.slice(0, 10)}.jsonl`);
        const location = await this.appendLine(file, message);
        this.rememberMessage(message, location);
      }
    }
    if (transaction.receipt) {
      if (!this.keys.has(transaction.receipt.key)) await this.appendLine(this.stateFile('sources.jsonl'), transaction.receipt);
      this.rememberReceipt(transaction.receipt);
    }
    for (const node of transaction.nodes) {
      const existing = this.nodes.get(keyOf(node.l, node.i));
      if (existing) {
        if (JSON.stringify(existing) !== JSON.stringify(node)) throw new Error('Recovery node differs');
      } else {
        validateNode(node);
        await this.appendLine(path.join(this.directory, 'tree', `${transaction.day}.jsonl`), node);
        this.nodes.set(keyOf(node.l, node.i), node); this.treeCount++;
      }
    }
    this.restore(transaction.checkpoint);
    await this.saveCheckpoint(transaction.checkpoint);
    await fs.rm(this.stateFile('pending.json'), { force: true });
    await syncDirectory(path.join(this.directory, 'state'));
  }
  async commit(messages, nodes, receipt) {
    const checkpoint = { ...this.checkpoint(), length: this.records.length + messages.length,
      treeCount: this.treeCount + nodes.length };
    const transaction = { version: 2, messages, nodes, receipt, day: new Date().toISOString().slice(0, 10), checkpoint };
    try {
      await atomic(this.stateFile('pending.json'), transaction);
      // Prepared nodes are already used for view calculation; applyTransaction appends them only once.
      for (const node of nodes) this.nodes.delete(keyOf(node.l, node.i));
      await this.applyTransaction(transaction);
    } catch (error) { this.writeFault = error; throw error; }
  }
  async readMessage(id) {
    const location = this.locations.get(id);
    if (!location) throw new Error('Invalid memory range');
    const handle = await fs.open(location.file, 'r');
    const buffer = Buffer.alloc(location.length);
    try {
      const { bytesRead } = await handle.read(buffer, 0, buffer.length, location.offset);
      if (bytesRead !== buffer.length) throw new Error('Incomplete archived message');
    } finally { await handle.close(); }
    return JSON.parse(buffer.toString('utf8'));
  }
  async readRecord(id) {
    this.checkRange(id, 1);
    const message = await this.readMessage(id);
    const meta = this.records[id];
    return { id, key: meta.key, kind: message.kind, text: message.text, date: message.date,
      threadId: meta.threadId, complete: meta.complete, size: message.size };
  }
  render(refs) {
    return `<chat>\n${refs.map(([l, i]) => {
      const node = this.nodes.get(keyOf(l, i));
      if (!node) throw new Error('Memory view is not ready: an earlier message is unsummarized');
      const { id, count } = range([l, i]);
      return `${id}+${count}|${node.text.replace(/[\r\n]/g, ' ')}`;
    }).join('\n')}\n</chat>`;
  }
  view() { return this.render(this.refs); }
  viewBytes(refs) {
    // Unknown leaves have no prompt text. The exact size is available when built.
    if (refs.some(([l, i]) => !this.nodes.has(keyOf(l, i)))) return null;
    return bytes(this.render(refs));
  }
  shrink(refs, max, min, active, T) {
    const size = this.viewBytes(refs);
    if (size === null) return active;
    if (size > max) active = true;
    if (!active) return false;
    while (bytes(this.render(refs)) > min) {
      let choice = -1, highest = -Infinity;
      for (let index = 0; index + 1 < refs.length; index++) {
        const [l, i] = refs[index], [otherL, otherI] = refs[index + 1];
        if (l !== otherL || i % 2 !== 0 || otherI !== i + 1 || !this.nodes.has(keyOf(l + 1, i / 2))) continue;
        const last = (i + 2) * 2 ** l - 1;
        const due = (T - last) / 2 ** l;
        if (due > highest) { choice = index; highest = due; }
      }
      if (choice < 0) return true;
      const [l, i] = refs[choice];
      refs.splice(choice, 2, [l + 1, i / 2]);
    }
    return false;
  }
  updateViews(length) {
    while (this.frontier < length && this.nodes.has(keyOf(0, this.frontier))) {
      this.compactionRefs.push([0, this.frontier]); this.frontier++;
    }
    const previouslyShrinking = this.shrinking;
    const previousRefs = JSON.stringify(this.refs);
    this.shrinking = this.shrink(this.refs, this.viewMaxBytes, this.viewMinBytes, this.shrinking, length);
    const chatMerged = previousRefs !== JSON.stringify(this.refs);
    if (chatMerged || (!previouslyShrinking && this.shrinking)) this.compactionShrinking = true;
    this.compactionShrinking = this.shrink(this.compactionRefs, this.compactionMaxBytes,
      this.compactionMinBytes, this.compactionShrinking, this.frontier);
  }
  addBuilt(ref, text, added) {
    const [l, i] = ref, key = keyOf(l, i);
    if (this.nodes.has(key)) return;
    const node = { l, i, text, size: bytes(text) };
    this.nodes.set(key, node); added.push(node);
    const sibling = this.nodes.get(keyOf(l, i + (i % 2 === 0 ? 1 : -1)));
    if (!sibling) return;
    const parent = [l + 1, Math.floor(i / 2)];
    if (this.nodes.has(keyOf(...parent))) return;
    const left = this.nodes.get(keyOf(l, i - i % 2));
    const right = this.nodes.get(keyOf(l, i - i % 2 + 1));
    const joined = `${left.text}\n${right.text}`;
    if (bytes(joined) <= this.nodeBytes) this.addBuilt(parent, joined, added);
    else this.enqueue(parent);
  }
  enqueue(ref) {
    const key = keyOf(...ref);
    if (this.nodes.has(key) || this.queued.has(key) || this.failed.has(key) || this.activeJobs.has(key)) return;
    this.queued.add(key);
    // Earlier task boundaries build their ancestors before later jobs need that context.
    const end = (ref[1] + 1) * 2 ** ref[0];
    let low = 0, high = this.queue.length;
    while (low < high) {
      const middle = (low + high) >>> 1, other = this.queue[middle];
      const otherEnd = (other[1] + 1) * 2 ** other[0];
      if (otherEnd < end || (otherEnd === end && other[0] > ref[0])) low = middle + 1;
      else high = middle;
    }
    this.queue.splice(low, 0, ref);
  }
  fillLeafWindow() {
    while (this.leafWindow.size < 8 && this.waitingLeaves.length) {
      const id = this.waitingLeaves.shift();
      if (this.nodes.has(keyOf(0, id))) continue;
      this.leafWindow.add(id); this.enqueue([0, id]);
    }
  }
  recoverQueues() {
    for (let id = 0; id < this.records.length; id++) if (!this.nodes.has(keyOf(0, id))) this.waitingLeaves.push(id);
    this.fillLeafWindow();
    // One startup traversal reconstructs ready work. Runtime completions unlock just their parent.
    for (const node of this.nodes.values()) {
      const parent = [node.l + 1, Math.floor(node.i / 2)];
      if (this.nodes.has(keyOf(node.l, node.i + (node.i % 2 === 0 ? 1 : -1)))) this.enqueue(parent);
    }
  }
  async append({ key, kind, text, date, threadId = '', complete = true }) {
    return this.serialize(async () => {
      if (typeof key !== 'string' || !key || !kinds.has(kind) || typeof text !== 'string' ||
          typeof threadId !== 'string' || typeof complete !== 'boolean' ||
          typeof date !== 'string' || !/^\d{4}-\d{2}-\d{2}T/.test(date) || !Number.isFinite(Date.parse(date)))
        throw new Error('Invalid source message');
      if (this.keys.has(key)) return this.readRecord(this.keys.get(key).id);
      const characters = Array.from(text);
      const parts = [];
      if (kind === 'echo' && characters.length > MAX_TEXT) {
        complete = false;
        parts.push(characters.slice(0, MAX_TEXT / 2).join('') + characters.slice(-MAX_TEXT / 2).join(''));
      } else {
        for (let at = 0; at < characters.length; at += MAX_TEXT) parts.push(characters.slice(at, at + MAX_TEXT).join(''));
        if (!parts.length) parts.push('');
      }
      const start = this.records.length, messages = [], added = [];
      for (const [index, part] of parts.entries()) {
        const message = { i: start + index, kind, text: part, size: bytes(part), date };
        messages.push(message); this.refs.push([0, message.i]);
        const source = `${kind}: ${part}`;
        if (bytes(source) <= this.nodeBytes) this.addBuilt([0, message.i], source, added);
        else this.waitingLeaves.push(message.i);
      }
      this.retryFailed();
      this.fillLeafWindow();
      this.updateViews(start + messages.length);
      await this.commit(messages, added, { key, id: start, count: messages.length, threadId, complete });
      return this.readRecord(start);
    });
  }
  prefixRefs(refs, end) {
    const result = [];
    const visit = ref => {
      const { id, count } = range(ref);
      if (id >= end) return;
      if (id + count <= end) { result.push(ref); return; }
      const [l, i] = ref;
      visit([l - 1, i * 2]); visit([l - 1, i * 2 + 1]);
    };
    for (const ref of refs) visit(ref);
    return result;
  }
  async prepare(ref) {
    const [l, i] = ref, { id, count } = range(ref);
    const end = Math.min(this.frontier, l === 0 ? id : id + count);
    const refs = this.prefixRefs(this.compactionRefs, end);
    this.shrink(refs, this.compactionMaxBytes, this.compactionMinBytes, false, end);
    const record = l === 0 ? await this.readRecord(id) : null;
    const source = record ? `${record.kind}: ${record.text}` :
      `${this.nodes.get(keyOf(l - 1, i * 2)).text}\n${this.nodes.get(keyOf(l - 1, i * 2 + 1)).text}`;
    return { source, context: { id, count, kind: record?.kind ?? 'merge', nodeBytes: this.nodeBytes,
      view: this.render(refs), endExclusive: end } };
  }
  compact(summarize, { shouldStop = () => false } = {}) {
    if (this.compacting) return this.compacting;
    if (typeof summarize !== 'function') return Promise.reject(new Error('Invalid summarizer'));
    const work = this.runCompaction(summarize, shouldStop);
    this.compacting = work.finally(() => { this.compacting = undefined; });
    return this.compacting;
  }
  async runCompaction(summarize, shouldStop) {
    let jobs = 0;
    const errors = [], running = new Set();
    const launch = async ref => {
      const key = keyOf(...ref);
      try {
        const { source, context } = await this.prepare(ref);
        const text = await summarize(source, context);
        if (typeof text !== 'string' || !text.trim()) throw new Error('Summary must be nonempty text');
        // The runtime retries in one conversation and returns its shortest candidate.
        // The gist permits a shortest candidate slightly over 512 bytes.
        await this.serialize(async () => {
          const added = [];
          this.addBuilt(ref, text, added);
          if (ref[0] === 0) { this.leafWindow.delete(ref[1]); this.fillLeafWindow(); }
          this.updateViews(this.records.length);
          await this.commit([], added);
          jobs++;
        });
      } catch (error) {
        errors.push(error);
        this.failed.set(key, { ref, epoch: this.appendEpoch, error });
      } finally { this.activeJobs.delete(key); }
    };
    for (;;) {
      if (!shouldStop()) {
        const refs = await this.serialize(() => {
          const selected = [];
          while (running.size + selected.length < 8 && this.queue.length) {
            const ref = this.queue.shift(), key = keyOf(...ref);
            this.queued.delete(key);
            if (this.nodes.has(key) || this.failed.has(key)) continue;
            this.activeJobs.add(key); selected.push(ref);
          }
          return selected;
        });
        for (const ref of refs) {
          const promise = launch(ref).finally(() => running.delete(promise));
          running.add(promise);
        }
      }
      if (!running.size) break;
      await Promise.race(running);
    }
    const result = { jobs, pending: this.queue.length + this.waitingLeaves.length + this.failed.size };
    if (errors.length) throw new AggregateError(errors, `Memory compaction failed: ${errors[0].message}`);
    return result;
  }
  retryFailed() {
    this.appendEpoch++;
    for (const [key, failure] of this.failed) {
      this.failed.delete(key);
      this.enqueue(failure.ref);
    }
  }
  async readyView(summarize) {
    // A new request arrives before its user message is appended. It is one retry opportunity.
    await this.serialize(async () => {
      if (!this.failed.size) return;
      this.retryFailed();
      await this.commit([], []);
    });
    try { await this.compact(summarize); }
    catch (error) { if (this.frontier !== this.records.length) throw error; }
    if (this.frontier !== this.records.length) {
      const failure = this.failed.values().next().value;
      throw new Error('Memory view is not ready; summaries made no progress', { cause: failure?.error });
    }
    return this.view();
  }
  checkRange(id, count) {
    if (!Number.isSafeInteger(id) || id < 0 || !Number.isSafeInteger(count) || count < 1 ||
        !Number.isInteger(Math.log2(count)) || id % count !== 0 || id + count > this.records.length)
      throw new Error('Invalid memory range');
  }
  async zoom(id, count = 1, offset = 0) {
    this.checkRange(id, count);
    if (count > 1) {
      const l = Math.log2(count) - 1, i = id / (count / 2);
      return { id, count, children: [i, i + 1].map(index => {
        const node = this.nodes.get(keyOf(l, index));
        if (!node) throw new Error('Memory node is not built');
        return { id: index * 2 ** l, count: 2 ** l, text: node.text, pending: false };
      }) };
    }
    const record = await this.readRecord(id), data = Buffer.from(record.text);
    if (!Number.isSafeInteger(offset) || offset < 0 || offset > data.length ||
        (offset < data.length && (data[offset] & 0xc0) === 0x80)) throw new Error('Invalid UTF-8 page offset');
    const text = prefix(data.subarray(offset).toString('utf8'), 12000), end = offset + bytes(text);
    return { id, count, kind: record.kind, text, offset, nextOffset: end < data.length ? end : null,
      totalBytes: data.length, complete: record.complete };
  }
  date(id) { this.checkRange(id, 1); return this.records[id].date; }
  async close() {
    await this.compacting?.catch(() => {});
    await this.chain;
    if (this.closed) return;
    this.closed = true;
    if (this.owner && this.owner.exitCode === null && this.owner.signalCode === null)
      await new Promise(resolve => { this.owner.once('exit', resolve); this.owner.stdin.end(); });
  }
}
