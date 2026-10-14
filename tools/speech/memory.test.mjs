import test from 'node:test';
import assert from 'node:assert/strict';
import { promises as fs } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { MemoryStore } from './memory.mjs';

const record = (key, text, extra = {}) => ({ key, kind: 'user', text,
  date: '2026-10-12T12:00:00Z', threadId: 'thread-1', ...extra });
async function fixture(t, options) {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'dojo-memory-v2-test-'));
  const store = await MemoryStore.open(directory, options);
  t.after(async () => { await store.close(); await fs.rm(directory, { recursive: true, force: true }); });
  return { directory, store };
}
const refsIn = view => view.split('\n').slice(1, -1).filter(Boolean).map(line => {
  const [, id, count] = /^(\d+)\+(\d+)\|/.exec(line);
  return [Number(id), Number(count)];
});
function coverage(view) {
  let cursor = 0;
  for (const [id, count] of refsIn(view)) {
    assert.equal(id, cursor);
    assert.equal(id % count, 0);
    assert.equal(Number.isInteger(Math.log2(count)), true);
    cursor += count;
  }
  return cursor;
}
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
async function until(predicate) {
  const deadline = Date.now() + 2000;
  while (!predicate()) {
    if (Date.now() > deadline) throw new Error('Condition did not arrive');
    await new Promise(resolve => setImmediate(resolve));
  }
}

test('daily exact archive, permanent IDs, deduplication, built short nodes, and private files', async t => {
  const { directory, store } = await fixture(t);
  const first = await store.append(record('one', 'cobalt'));
  assert.equal(first.id, 0);
  assert.equal(first.text, 'cobalt');
  assert.equal((await store.append(record('one', 'redelivery must not overwrite'))).text, 'cobalt');
  await store.append(record('two', 'orange', { date: '2026-10-13T01:00:00Z', kind: 'unii' }));
  assert.equal(store.view(), '<chat>\n0+1|user: cobalt\n1+1|unii: orange\n</chat>');
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(directory, 'main/2026-10-12.jsonl'))),
    { i: 0, kind: 'user', text: 'cobalt', size: 6, date: '2026-10-12T12:00:00Z' });
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(directory, 'main/2026-10-13.jsonl'))),
    { i: 1, kind: 'unii', text: 'orange', size: 6, date: '2026-10-13T01:00:00Z' });
  assert.deepEqual(JSON.parse(await fs.readFile(path.join(directory, 'view.json'))), [[0, 0], [0, 1]]);
  assert.deepEqual((await store.zoom(0, 2)).children.map(node => node.text), ['user: cobalt', 'unii: orange']);
  assert.equal(store.date(1), '2026-10-13T01:00:00Z');
  assert.equal(store.records[0].text, undefined);
  for (const name of ['main/2026-10-12.jsonl', 'view.json', 'state/checkpoint.json'])
    assert.equal((await fs.stat(path.join(directory, name))).mode & 0o777, 0o600);
  assert.equal((await fs.stat(directory)).mode & 0o777, 0o700);
  await assert.rejects(MemoryStore.open(directory), /already owned/);
});

test('short source boundary is 512 UTF-8 bytes and no pending material enters a view', async t => {
  const { store } = await fixture(t);
  await store.append(record('fits', 'é'.repeat(253))); // kind prefix plus text equals 512.
  const built = store.view();
  assert.equal(built, `<chat>\n0+1|user: ${'é'.repeat(253)}\n</chat>`);
  await store.append(record('long', 'é'.repeat(254)));
  assert.throws(() => store.view(), /not ready/);
  let calls = 0;
  const result = await store.readyView((source, context) => {
    calls++;
    if (context.count === 1) {
      assert.equal(context.kind, 'user');
      assert.equal(source, `user: ${'é'.repeat(254)}`);
      assert.equal(context.view, built);
      assert.equal(context.nodeBytes, 512);
      return 'user: a longer accented message';
    }
    return 'user: accented messages';
  });
  assert.equal(calls, 2);
  assert.equal(result, `<chat>\n0+1|user: ${'é'.repeat(253)}\n1+1|user: a longer accented message\n</chat>`);
});

test('non-tool long text splits losslessly; echo output clips head and tail by Unicode character', async t => {
  const { store } = await fixture(t);
  const text = '🙂'.repeat(15_001) + 'M'.repeat(15_000) + '終'.repeat(15_001);
  const first = await store.append(record('user-long', text));
  assert.equal(first.id, 0);
  assert.equal(store.records.length, 2);
  assert.equal((await store.readRecord(0)).text + (await store.readRecord(1)).text, text);
  await store.append(record('user-long', 'duplicate'));
  assert.equal(store.records.length, 2);
  await store.append(record('output', text, { kind: 'echo' }));
  const output = await store.readRecord(2);
  assert.equal(output.text, '🙂'.repeat(15_000) + '終'.repeat(15_000));
  assert.equal(output.complete, false);
  await store.append(record('already-incomplete', 'partial provider result', { kind: 'echo', complete: false }));
  assert.equal((await store.readRecord(3)).complete, false);
  assert.equal((await store.zoom(3)).complete, false);
  let offset = 0, recovered = '';
  do {
    const page = await store.zoom(2, 1, offset);
    assert.equal(page.complete, false);
    assert.ok(Buffer.byteLength(page.text) <= 12000);
    assert.equal(page.text.includes('\ufffd'), false);
    recovered += page.text; offset = page.nextOffset;
  } while (offset !== null);
  assert.equal(recovered, output.text);
  await assert.rejects(store.zoom(2, 1, 1), /UTF-8/);
  await assert.rejects(store.zoom(4), /range/);
});

test('eight workers, shared compact promise, leaf admission gate and first-unbuilt context cutoff', async t => {
  const { store } = await fixture(t);
  for (let id = 0; id < 12; id++) await store.append(record(`source-${id}`, `${id} ${'x'.repeat(700)}`));
  const calls = [], releases = new Map();
  let active = 0, maximum = 0, drain = false;
  const summarize = async (source, context) => {
    const hold = deferred();
    calls.push(context); releases.set(`${context.id}+${context.count}`, hold);
    maximum = Math.max(maximum, ++active);
    try { return drain ? 's'.repeat(300) : await hold.promise; } finally { active--; }
  };
  const work = store.compact(summarize);
  assert.equal(store.compact(summarize), work);
  try {
  await until(() => calls.length === 8);
  assert.deepEqual(calls.map(call => call.id).sort((a, b) => a - b), [0, 1, 2, 3, 4, 5, 6, 7]);
  assert.equal(maximum, 8);
  assert.equal(calls.every(call => call.view === '<chat>\n\n</chat>'), true);
  // A completion after the missing first leaf may admit one more job, but its context remains empty.
  releases.get('7+1').resolve('s'.repeat(300));
  await until(() => calls.some(call => call.id === 8));
  assert.equal(calls.find(call => call.id === 8).endExclusive, 0);
  assert.equal(calls.find(call => call.id === 8).view, '<chat>\n\n</chat>');
  // Resolve every admitted job until its newly eligible parents also finish.
  let finished = false;
  work.finally(() => { finished = true; });
  while (!finished) {
    for (const hold of releases.values()) hold.resolve('s'.repeat(300));
    await new Promise(resolve => setImmediate(resolve));
  }
  await work;
  assert.equal(maximum, 8);
  assert.equal(coverage(store.view()), 12);
  for (const context of calls) {
    const ranges = refsIn(context.view);
    assert.ok(ranges.every(([id, count]) => id + count <= context.endExclusive));
  }
  assert.equal((await store.compact(() => { throw new Error('must not rebuild'); })).jobs, 0);
  } finally {
    drain = true;
    for (const hold of releases.values()) hold.resolve('s'.repeat(300));
    await work;
  }
});

test('background failures wait for an append opportunity instead of retrying in compact', async t => {
  const { store } = await fixture(t);
  await store.append(record('long', 'source '.repeat(200)));
  await assert.rejects(store.readyView(() => { throw new Error('provider failed'); }), /provider failed/);
  assert.deepEqual(await store.compact(() => 'must wait for a message'), { jobs: 0, pending: 1 });
  await store.append(record('next', 'next'));
  assert.equal(await store.readyView(() => 'Recovered source.'), '<chat>\n0+1|Recovered source.\n1+1|user: next\n</chat>');
});

test('default foreground batch crosses 128000 and shrinks to 64000; compaction view stays 16000-32000', async t => {
  const { directory, store } = await fixture(t);
  const body = 'x'.repeat(494); // 500-byte short leaves.
  for (let id = 0; id < 250; id++) await store.append(record(`${id}`, body));
  const before = store.view();
  assert.ok(Buffer.byteLength(before) < 128000);
  assert.equal(refsIn(before).every(([, count]) => count === 1), true);
  for (let id = 250; id < 265; id++) await store.append(record(`${id}`, body));
  assert.ok(Buffer.byteLength(store.view()) > 128000); // Parents are not built yet.
  const contexts = [];
  await store.readyView((source, context) => { contexts.push(context); return 'z'.repeat(500); });
  assert.ok(Buffer.byteLength(store.view()) <= 64000);
  assert.equal(coverage(store.view()), 265);
  assert.ok(contexts.every(context => Buffer.byteLength(context.view) <= 32000));
  const compactRefs = JSON.parse(await fs.readFile(path.join(directory, 'compaction-view.json')));
  assert.ok(compactRefs.some(([level]) => level > 0));
  const saved = JSON.parse(await fs.readFile(path.join(directory, 'state/checkpoint.json')));
  assert.equal(saved.shrinking, false);
  const bounded = store.view();
  await store.append(record('last', 'final fact'));
  assert.equal(store.view(), bounded.replace('\n</chat>', '\n265+1|user: final fact\n</chat>'));
});

test('merge age uses pair last message, giving recent sibling pair priority over old large pair', async t => {
  const { directory, store } = await fixture(t);
  for (let id = 0; id < 10; id++) await store.append(record(`${id}`, 'x'.repeat(260)));
  await store.readyView(() => 'm'.repeat(200));
  await store.close();
  const checkpointPath = path.join(directory, 'state/checkpoint.json');
  const saved = JSON.parse(await fs.readFile(checkpointPath));
  saved.refs = [[2, 0], [2, 1], [0, 8], [0, 9]];
  await fs.writeFile(checkpointPath, JSON.stringify(saved));
  await fs.writeFile(path.join(directory, 'view.json'), JSON.stringify(saved.refs));
  const reopened = await MemoryStore.open(directory, { viewMaxBytes: 1500, viewMinBytes: 1400 });
  t.after(() => reopened.close());
  await reopened.append(record('10', 'x'.repeat(260)));
  assert.deepEqual(refsIn(reopened.view()), [[0, 4], [4, 4], [8, 2], [10, 1]]);
});

test('saved views survive restart byte-for-byte and nodes never rebuild; daily file order does not renumber IDs', async t => {
  const { directory, store } = await fixture(t);
  await store.append(record('new-date', 'x'.repeat(700), { date: '2026-10-14T12:00:00Z' }));
  await store.append(record('old-date', 'y'.repeat(700)));
  await store.readyView((source, context) => context.count === 1 ? `Summary ${context.id}` : 'Merged');
  const before = store.view();
  const savedViews = await Promise.all(['view.json', 'compaction-view.json'].map(name => fs.readFile(path.join(directory, name), 'utf8')));
  await store.close();
  const reopened = await MemoryStore.open(directory);
  t.after(() => reopened.close());
  assert.equal(reopened.view(), before);
  assert.deepEqual(await Promise.all(['view.json', 'compaction-view.json'].map(name => fs.readFile(path.join(directory, name), 'utf8'))), savedViews);
  assert.equal((await reopened.append(record('new-date', 'changed'))).text, 'x'.repeat(700));
  assert.deepEqual(await reopened.compact(() => { throw new Error('rebuilt'); }), { jobs: 0, pending: 0 });
  assert.equal((await reopened.append(record('after', 'after restart'))).id, 2);
});

test('recovery completes a prepared append, preserving saved prefix refs instead of rebuilding its view', async t => {
  const { directory, store } = await fixture(t);
  await store.append(record('first', 'first'));
  await store.close();
  const checkpoint = JSON.parse(await fs.readFile(path.join(directory, 'state/checkpoint.json')));
  const message = { i: 1, kind: 'user', text: 'second', size: 6, date: '2026-10-12T12:00:00Z' };
  const node = { l: 0, i: 1, text: 'user: second', size: 12 };
  const parentText = 'user: first\nuser: second';
  const parent = { l: 1, i: 0, text: parentText, size: Buffer.byteLength(parentText) };
  const pending = { version: 2, messages: [message], nodes: [node, parent], day: '2026-10-12',
    receipt: { key: 'second', id: 1, count: 1, threadId: 'thread-1', complete: true },
    checkpoint: { ...checkpoint, length: 2, treeCount: 3, refs: [[0, 0], [0, 1]],
      compactionRefs: [[0, 0], [0, 1]], frontier: 2, appendEpoch: 2 } };
  await fs.writeFile(path.join(directory, 'state/pending.json'), JSON.stringify(pending));
  await fs.appendFile(path.join(directory, 'main/2026-10-12.jsonl'), JSON.stringify(message) + '\n');
  const reopened = await MemoryStore.open(directory);
  t.after(() => reopened.close());
  assert.equal(reopened.view(), '<chat>\n0+1|user: first\n1+1|user: second\n</chat>');
  assert.equal((await reopened.append(record('second', 'duplicate'))).text, 'second');
  assert.equal((await reopened.append(record('third', 'third'))).id, 2);
  assert.equal((await fs.readFile(path.join(directory, 'main/2026-10-12.jsonl'), 'utf8')).split('\n').length, 4);
});

test('a missing view checkpoint fails closed; trailing incomplete append repairs only its tail', async t => {
  const { directory, store } = await fixture(t);
  await store.append(record('first', 'original'));
  await store.close();
  const file = path.join(directory, 'main/2026-10-12.jsonl');
  const before = await fs.readFile(file, 'utf8');
  await fs.appendFile(file, '{"i":1,"torn');
  const reopened = await MemoryStore.open(directory);
  assert.equal((await reopened.zoom(0)).text, 'original');
  assert.equal(await fs.readFile(file, 'utf8'), before);
  await reopened.close();
  await fs.unlink(path.join(directory, 'state/checkpoint.json'));
  await assert.rejects(MemoryStore.open(directory), /Missing persisted view/);
  assert.equal(await fs.readFile(file, 'utf8'), before);
});

test('an earlier complete corrupt line fails open without modifying the source', async t => {
  const { directory, store } = await fixture(t);
  await store.close();
  const file = path.join(directory, 'main/2026-10-12.jsonl');
  const corrupt = '{broken}\n{"unfinished":';
  await fs.writeFile(file, corrupt);
  await assert.rejects(MemoryStore.open(directory), SyntaxError);
  assert.equal(await fs.readFile(file, 'utf8'), corrupt);
});

test('append while summaries run preserves newest records and readiness waits for them', async t => {
  const { store } = await fixture(t);
  await store.append(record('long', 'x'.repeat(700)));
  const started = deferred(), release = deferred();
  const work = store.compact(async () => { started.resolve(); return release.promise; });
  await started.promise;
  const appended = await store.append(record('during', 'during summary'));
  assert.equal(appended.id, 1);
  release.resolve('Long source summary');
  await work;
  assert.equal(store.view(), '<chat>\n0+1|Long source summary\n1+1|user: during summary\n</chat>');
});

test('OS lease releases after crash and losing ownership refuses new writes', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'dojo-memory-v2-crash-'));
  t.after(() => fs.rm(directory, { recursive: true, force: true }));
  const module = new URL('./memory.mjs', import.meta.url).href;
  const child = spawn(process.execPath, ['--input-type=module', '-e', `
    import { MemoryStore } from ${JSON.stringify(module)};
    const store = await MemoryStore.open(${JSON.stringify(directory)});
    await store.append(${JSON.stringify(record('crashed', 'durable before crash'))});
    process.stdout.write('ready\\n'); setInterval(() => {}, 1000);
  `], { stdio: ['ignore', 'pipe', 'pipe'] });
  await new Promise((resolve, reject) => { child.stdout.once('data', resolve); child.once('error', reject);
    child.once('exit', () => reject(new Error('Child exited before append'))); });
  const exited = new Promise(resolve => child.once('exit', resolve));
  child.kill('SIGKILL'); await exited;
  const store = await MemoryStore.open(directory);
  t.after(() => store.close());
  assert.equal((await store.zoom(0)).text, 'durable before crash');
  const leaseExit = new Promise(resolve => store.owner.once('exit', resolve));
  store.owner.kill('SIGKILL'); await leaseExit;
  await assert.rejects(store.append(record('after', 'must not write')), /ownership was lost/);
  assert.equal((await store.zoom(0)).text, 'durable before crash');
});

test('unfinished shrink targets survive restart and only built parents can enter the view', async t => {
  const { directory, store } = await fixture(t, { viewMaxBytes: 1500, viewMinBytes: 800 });
  for (let id = 0; id < 4; id++) await store.append(record(`${id}`, 'x'.repeat(494)));
  const before = store.view();
  assert.deepEqual(refsIn(before), [[0, 1], [1, 1], [2, 1], [3, 1]]);
  assert.equal(JSON.parse(await fs.readFile(path.join(directory, 'state/checkpoint.json'))).shrinking, true);
  await store.close();
  const reopened = await MemoryStore.open(directory, { viewMaxBytes: 1500, viewMinBytes: 800 });
  t.after(() => reopened.close());
  assert.equal(reopened.view(), before);
  await reopened.readyView(() => 'summary');
  assert.ok(Buffer.byteLength(reopened.view()) <= 800);
  assert.equal(coverage(reopened.view()), 4);
  assert.equal(JSON.parse(await fs.readFile(path.join(directory, 'state/checkpoint.json'))).shrinking, false);
  assert.deepEqual(refsIn(reopened.view()), [[0, 2], [2, 2]]);
});

test('oversized shortest candidates retain real sizes instead of cutting source facts', async t => {
  const { directory, store } = await fixture(t);
  await store.append(record('long', 'x'.repeat(700)));
  await store.readyView(() => 'é'.repeat(257));
  assert.equal(store.view(), `<chat>\n0+1|${'é'.repeat(257)}\n</chat>`);
  const lines = (await fs.readFile(path.join(directory, 'tree', `${new Date().toISOString().slice(0, 10)}.jsonl`), 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(lines, [{ l: 0, i: 0, text: 'é'.repeat(257), size: 514 }]);
});

test('stopped compaction leaves ready work untouched and concurrent appends deduplicate IDs', async t => {
  const { store } = await fixture(t);
  const results = await Promise.all([store.append(record('a', 'x'.repeat(700))), store.append(record('b', 'b')), store.append(record('a', 'changed'))]);
  assert.deepEqual(results.map(item => item.id), [0, 1, 0]);
  assert.equal((await store.readRecord(0)).text, 'x'.repeat(700));
  assert.deepEqual(await store.compact(() => { throw new Error('stopped'); }, { shouldStop: () => true }), { jobs: 0, pending: 1 });
  assert.equal(await store.readyView(() => 'Source summary.'), '<chat>\n0+1|Source summary.\n1+1|user: b\n</chat>');
});

test('owned archive directories reject symlinks without touching their targets', async t => {
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), 'dojo-memory-owned-'));
  const target = await fs.mkdtemp(path.join(os.tmpdir(), 'dojo-memory-target-'));
  t.after(async () => { await fs.rm(directory, { recursive: true, force: true }); await fs.rm(target, { recursive: true, force: true }); });
  await fs.writeFile(path.join(target, 'sentinel'), 'untouched');
  await fs.symlink(target, path.join(directory, 'main'));
  await assert.rejects(MemoryStore.open(directory), /symlink/);
  assert.equal(await fs.readFile(path.join(target, 'sentinel'), 'utf8'), 'untouched');
  assert.deepEqual(await fs.readdir(target), ['sentinel']);
});


test('a new readyView retries failed leaves once without logging the next user message first', async t => {
  const { directory, store } = await fixture(t);
  await store.append(record('prior-response', 'source '.repeat(200)));
  let attempts = 0;
  const failing = () => { attempts++; throw new Error('provider unavailable'); };
  await assert.rejects(store.readyView(failing), /provider unavailable/);
  assert.equal(attempts, 1);
  await assert.rejects(store.readyView(failing), /provider unavailable/);
  assert.equal(attempts, 2);
  assert.deepEqual(await store.compact(failing), { jobs: 0, pending: 1 });
  assert.equal(attempts, 2);
  const ready = await store.readyView(() => { attempts++; return 'Recovered previous response.'; });
  assert.equal(ready, '<chat>\n0+1|Recovered previous response.\n</chat>');
  assert.equal(attempts, 3);
  assert.equal(store.records.length, 1);
  assert.equal((await fs.readFile(path.join(directory, 'main/2026-10-12.jsonl'), 'utf8')).trim().split('\n').length, 1);
  assert.equal(JSON.parse(await fs.readFile(path.join(directory, 'state/checkpoint.json'))).appendEpoch, 3);
});


test('eight failed workers each get one attempt per readyView opportunity', async t => {
  const { store } = await fixture(t);
  for (let id = 0; id < 8; id++) await store.append(record(`${id}`, 'source '.repeat(200)));
  let attempts = 0;
  const failing = () => { attempts++; throw new Error('offline'); };
  await assert.rejects(store.readyView(failing), /offline/);
  assert.equal(attempts, 8);
  await assert.rejects(store.readyView(failing), /offline/);
  assert.equal(attempts, 16);
  assert.deepEqual(await store.compact(failing), { jobs: 0, pending: 8 });
  assert.equal(attempts, 16);
  assert.equal(coverage(await store.readyView(() => 'Recovered.')), 8);
});
