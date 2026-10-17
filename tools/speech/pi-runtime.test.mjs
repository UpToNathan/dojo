import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall, fauxThinking } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { PiRuntime, summaryTask } from './pi-runtime.mjs';

async function fixture(t, responses, overrides = {}) {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-test-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider); faux.setResponses(responses);
  const records = new Map(), calls = [];
  const memory = { async append(record) { records.set(record.key, record); }, async readyView() { calls.push('ready'); return '<chat>\n0+1|old memory\n</chat>'; }, async zoom() { calls.push('zoom'); return 'original'; }, async date() { return '2026-01-01'; } };
  const events = [];
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory, config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'test', instructions: 'Fixed instructions', desktopTools: false, ...overrides }, onEvent: event => events.push(event) });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  return { runtime, faux, memory, records, calls, events, stateDir, models };
}

async function idle(runtime) { await runtime.queue; await runtime.conversation.waitForIdle(runtime.context); await runtime.archived; }

test('the runtime starts without an account and fails a request clearly instead of routing to a fallback', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-no-account-'));
  const events = [];
  const credentials = { backend: 'file', read: async () => undefined, importAccount: async () => { throw new Error('Not signed in.'); },
    importCandidate: async () => { throw new Error('Not signed in.'); } };
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { append: async () => {}, readyView: async () => '<chat>\n</chat>' },
    config: { storage: new MemoryStorage(), credentials, instructions: 'Fixed', desktopTools: false }, onEvent: event => events.push(event) });
  try {
    assert.equal(runtime.selection, null);
    await runtime.submit('Do not route', { requestId: 'no-account' });
    await runtime.queue;
    assert(events.some(event => event.type === 'run' && event.status === 'error' && /Sign in to use Dojo|Not signed in/.test(event.error)));
    assert.equal((await runtime.state()).admissions['no-account'].status, 'failed');
    assert.equal(runtime.agent.model.provider, 'openai-codex');
  } finally { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); }
});

test('a locked keyring raises the unlock dialog when a command starts, then the request proceeds', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-locked-keyring-'));
  const events = []; let locked = true, unlocks = 0;
  const credentials = { backend: 'keyring',
    read: async id => { if (locked) throw new Error('The Dojo keyring is locked. Unlock it in the system dialog, then retry.'); return id === 'opencode-go' ? { type: 'api_key', key: 'k' } : undefined; },
    unlock: async () => { unlocks++; locked = false; return true; },
    importAccount: async () => {}, importCandidate: async () => { throw new Error('Not signed in.'); } };
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { append: async () => {}, readyView: async () => { throw new Error('stop after unlock'); } },
    config: { storage: new MemoryStorage(), credentials, instructions: 'Fixed', desktopTools: false }, onEvent: event => events.push(event) });
  try {
    assert.equal(runtime.selection, null);
    await runtime.submit('Open T3 code', { requestId: 'locked' });
    await runtime.queue;
    assert.equal(unlocks, 1);
    assert(events.some(event => event.type === 'run' && event.status === 'error' && /stop after unlock/.test(event.error)), JSON.stringify(events));
    assert.equal(runtime.selectionAvailable, true);
  } finally { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); }
});

test('fresh foreground context has prior view, stable tools, logs calls/results and no thoughts', async t => {
  const requests = [];
  const { runtime, records, calls } = await fixture(t, [
    context => { requests.push(context); return fauxAssistantMessage([fauxThinking('private'), fauxToolCall('zoom', { id: 0, n: 1 })], { stopReason: 'toolUse' }); },
    context => { requests.push(context); return fauxAssistantMessage('First answer'); },
    context => { requests.push(context); return fauxAssistantMessage('Second answer'); },
  ]);
  await runtime.submit('First question', { requestId: 'first' }); await idle(runtime);
  await runtime.submit('Second question', { requestId: 'second' }); await idle(runtime);
  assert.equal(calls.filter(x => x === 'ready').length, 2);
  const last = JSON.stringify(requests.at(-1));
  assert.match(last, /old memory/); assert.match(last, /Second question/); assert.doesNotMatch(last, /First question|First answer/);
  const archived = [...records.values()];
  assert(archived.some(x => x.kind === 'tool' && x.text.includes('zoom')));
  assert(archived.some(x => x.kind === 'echo' && x.text.includes('original')));
  assert(!archived.some(x => x.text.includes('private')));
  const duplicate = await runtime.submit('First question', { requestId: 'first' }); assert.equal(duplicate.duplicate, true);
});

test('summary uses same tools and prompt, blocks execution, repairs five times and keeps shortest', async t => {
  const requests = [];
  const capture = text => context => { requests.push(context); return fauxAssistantMessage(text); };
  const { runtime, calls } = await fixture(t, [
    context => { requests.push(context); return fauxAssistantMessage(fauxToolCall('zoom', { id: 0, n: 1 }), { stopReason: 'toolUse' }); },
    capture('a'.repeat(600)), capture('b'.repeat(550)), capture('c'.repeat(580)), capture('d'.repeat(530)), capture('e'.repeat(540)),
  ]);
  const answer = await runtime.summarize('user: whole input', { kind: 'leaf', id: 5, view: '<chat>\n0+4|summary context\n</chat>' });
  assert.equal(answer, 'd'.repeat(530)); assert(!calls.includes('zoom'));
  assert.match(JSON.stringify(requests[0]), /summary context/);
  assert.match(JSON.stringify(requests.at(-1)), /Too long/);
  assert.equal(requests.length, 6);
  assert.deepEqual(requests[0].tools?.map(x => x.name), requests.at(-1).tools?.map(x => x.name));
  assert.equal(requests[0].systemPrompt, requests.at(-1).systemPrompt);
});

test('summary task uses a 512 dash ruler and exact merge range', () => {
  const task = summaryTask('a\nb', { kind: 'merge', ref: [3, 5] });
  assert(task.includes('-'.repeat(512))); assert.match(task, /40\+4 and 44\+4/); assert.match(task, /40 to 47/);
});

test('model selection switches only subsequent idle requests and not the request already completed', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-model-switch-'));
  const faux = fauxProvider({ models: [{ id: 'model-a' }, { id: 'model-b' }] });
  const models = createModels(); models.setProvider(faux.provider);
  const used = [];
  let releaseFirst, firstStarted;
  const firstGate = new Promise(resolve => { releaseFirst = resolve; });
  const began = new Promise(resolve => { firstStarted = resolve; });
  faux.setResponses([async (context, options, state, model) => { used.push(model.id); firstStarted(); await firstGate; return fauxAssistantMessage('first'); },
    (context, options, state, model) => { used.push(model.id); return fauxAssistantMessage('second'); }]);
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { async append() {}, async readyView() { return '<chat>\n</chat>'; } },
    config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'model-a', instructions: 'Fixed', desktopTools: false } });
  try {
    await runtime.submit('First', { requestId: 'model-a' }); await began;
    await runtime.setModelSelection({ account: 'faux', model: 'model-b' }, { available: true, summaryModelId: 'model-b' });
    assert.equal(runtime.agent.model.modelId, 'model-a');
    releaseFirst(); await idle(runtime);
    await runtime.submit('Second', { requestId: 'model-b' }); await idle(runtime);
    assert.deepEqual(used, ['model-a', 'model-b']);
  } finally { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); }
});

test('active input steers the same run between tool calls without a second memory view', async t => {
  let release, started;
  const gate = new Promise(resolve => { release = resolve; });
  const began = new Promise(resolve => { started = resolve; });
  let finalContext;
  const { runtime, calls, records } = await fixture(t, [
    async () => { started(); await gate; return fauxAssistantMessage(fauxToolCall('zoom', { id: 0, n: 1 }), { stopReason: 'toolUse' }); },
    context => { finalContext = context; return fauxAssistantMessage('Steering received'); },
  ]);
  await runtime.submit('Begin work', { requestId: 'begin' }); await began;
  const receipt = await runtime.submit('Use the correction', { requestId: 'steer' });
  assert.equal(receipt.conversationId, runtime.id);
  await runtime.queue; release(); await idle(runtime);
  assert.equal(calls.filter(x => x === 'ready').length, 1);
  assert.match(JSON.stringify(finalContext), /Use the correction/);
  assert.equal([...records.values()].filter(x => x.kind === 'user').length, 2);
});

test('SQLite reopen preserves conversation identity, deduplicates admissions and archives', async t => {
  const { runtime, models, memory, records, stateDir } = await fixture(t, [fauxAssistantMessage('Persisted answer')], { storage: undefined });
  await runtime.submit('Persist this', { requestId: 'persist' }); await idle(runtime);
  const id = runtime.id, count = records.size;
  await runtime.close();
  const reopened = await PiRuntime.open({ stateDir, root: process.cwd(), memory, config: { models, provider: 'faux', model: 'test', instructions: 'Fixed instructions', desktopTools: false } });
  t.after(() => reopened.close());
  assert.equal(reopened.id, id);
  assert.equal((await reopened.submit('Persist this', { requestId: 'persist' })).duplicate, true);
  assert.equal(records.size, count);
});


test('a process crash during generation resumes the durable run without duplicate input', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-crash-'));
  t.after(() => rm(stateDir, { recursive: true, force: true }));
  const script = `
    import { createModels } from '@earendil-works/pi-ai/models';
    import { fauxProvider } from '@earendil-works/pi-ai/providers/faux';
    import { PiRuntime } from './pi-runtime.mjs';
    const faux = fauxProvider({ models: [{ id: 'test' }] });
    const models = createModels(); models.setProvider(faux.provider);
    faux.setResponses([() => { process.exit(0); }]);
    const runtime = await PiRuntime.open({ stateDir: process.argv[1], root: process.cwd(), memory: { append: async () => {}, readyView: async () => '<chat>\\n</chat>' }, config: { models, provider: 'faux', model: 'test', instructions: 'Fixed', desktopTools: false } });
    await runtime.submit('Crash request', { requestId: 'crash' });
    await runtime.queue;
    await runtime.conversation.waitForIdle(runtime.context);
  `;
  execFileSync(process.execPath, ['--input-type=module', '--eval', script, stateDir], { cwd: fileURLToPath(new URL('.', import.meta.url)) });
  const faux = fauxProvider({ models: [{ id: 'test' }] }); const models = createModels(); models.setProvider(faux.provider);
  faux.setResponses([fauxAssistantMessage('Recovered answer')]);
  const records = new Map();
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { append: async record => records.set(record.key, record), readyView: async () => '<chat>\n</chat>' }, config: { models, provider: 'faux', model: 'test', instructions: 'Fixed', desktopTools: false } });
  t.after(() => runtime.close());
  runtime.resume(); await idle(runtime);
  assert([...records.values()].some(x => x.kind === 'unii' && x.text === 'Recovered answer'));
  assert.equal((await runtime.submit('Crash request', { requestId: 'crash' })).duplicate, true);
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  assert.equal(entries.items.filter(x => x.kind === 'pi.user').length, 1);
});

test('zoom retrieves original durable images only for an owned result leaf first page', async t => {
  let retrieved;
  const image = { type: 'image', data: 'iVBORw0KGgo=', mimeType: 'image/png' };
  const { runtime, memory } = await fixture(t, [
    fauxAssistantMessage(fauxToolCall('zoom', { id: 7, n: 1 }), { stopReason: 'toolUse' }),
    context => { retrieved = context.messages.find(message => message.role === 'toolResult' && message.toolName === 'zoom'); return fauxAssistantMessage('Image read'); },
  ]);
  const source = await runtime.conversation.commit(tx => tx.appendEntry(runtime.pi.ToolResultEntry, runtime.id, { model: [{ role: 'toolResult', toolCallId: 'screenshot', toolName: 'desktop_screenshot', content: [{ type: 'text', text: 'Screenshot coordinates' }, image], isError: false, timestamp: 1 }], data: { diagnostics: [] } }), runtime.context);
  memory.readRecord = async () => ({ key: `pi:${runtime.id}:${source.id}:result` });
  await runtime.submit('Read original screenshot', { requestId: 'image' }); await idle(runtime);
  assert.deepEqual(retrieved.content.find(block => block.type === 'image'), image);
  assert.equal((await runtime.zoom({ id: 7, n: 1, offset: 10 })).content.filter(block => block.type === 'image').length, 0);
  assert.equal((await runtime.zoom({ id: 0, n: 8 })).content.filter(block => block.type === 'image').length, 0);
  memory.readRecord = async () => ({ key: `pi:${runtime.id + 1}:${source.id}:result` });
  assert.equal((await runtime.zoom({ id: 7, n: 1 })).content.filter(block => block.type === 'image').length, 0);
});

test('a failed model call reports the error instead of ending the run silently', async t => {
  const { runtime, events } = await fixture(t, [fauxAssistantMessage([], { stopReason: 'error', errorMessage: 'Connection error.' })]);
  await runtime.submit('Navigate somewhere', { requestId: 'fails' }); await idle(runtime);
  assert(events.some(event => event.type === 'run' && event.status === 'error' && event.error === 'Connection error.'));
});

test('fresh view explicitly archives a committed final reply when the watcher lags', async t => {
  let nextContext;
  const { runtime, memory, records } = await fixture(t, [fauxAssistantMessage('Final result before reset'), context => { nextContext = context; return fauxAssistantMessage('Next response'); }]);
  await runtime.watch.stop();
  memory.readyView = async () => `<chat>\n${[...records.values()].filter(record => record.kind === 'unii').map(record => record.text).join('\n')}\n</chat>`;
  await runtime.submit('First', { requestId: 'lag-first' }); await idle(runtime);
  assert(![...records.values()].some(record => record.text === 'Final result before reset'));
  await runtime.submit('Next', { requestId: 'lag-next' }); await idle(runtime);
  assert.match(JSON.stringify(nextContext), /Final result before reset/);
});

test('shutdown cancels a stalled summary and preserves pending foreground work', { timeout: 3000 }, async t => {
  let started;
  const began = new Promise(resolve => { started = resolve; });
  const { runtime } = await fixture(t, [async (_context, options) => { started(); return await new Promise(resolve => options.signal.addEventListener('abort', () => resolve(fauxAssistantMessage('', { stopReason: 'aborted' })), { once: true })); }]);
  const summary = runtime.summarize('user: stalled summary');
  const rejection = assert.rejects(summary, /closed/);
  await began;
  await runtime.close();
  await rejection;
  assert.equal(runtime.summaryJobs.size, 0);
});

test('summary deadline aborts the conversation and releases its timer', { timeout: 3000 }, async t => {
  const { runtime } = await fixture(t, [async (_context, options) => await new Promise(resolve => options.signal.addEventListener('abort', () => resolve(fauxAssistantMessage('', { stopReason: 'aborted' })), { once: true }))], { summaryTimeoutMs: 20 });
  await assert.rejects(runtime.summarize('user: stalled summary'), /timed out/);
  assert.equal(runtime.summaryJobs.size, 0);
});

test('steering drains older committed tools before the user record while the watcher lags', async t => {
  let releaseFirst, releaseFinal, firstStarted, finalStarted;
  const firstGate = new Promise(resolve => { releaseFirst = resolve; });
  const finalGate = new Promise(resolve => { releaseFinal = resolve; });
  const began = new Promise(resolve => { firstStarted = resolve; });
  const toolsCommitted = new Promise(resolve => { finalStarted = resolve; });
  const { runtime, records, calls } = await fixture(t, [
    async () => { firstStarted(); await firstGate; return fauxAssistantMessage(fauxToolCall('zoom', { id: 0, n: 1 }), { stopReason: 'toolUse' }); },
    async () => { finalStarted(); await finalGate; return fauxAssistantMessage('Original work finished'); },
    fauxAssistantMessage('Steering received'),
  ]);
  await runtime.submit('Begin', { requestId: 'ordered-begin' }); await began; await runtime.queue;
  await runtime.watch.stop();
  runtime.live = await runtime.harness.snapshot(runtime.pi.LiveDoc, runtime.id, runtime.context);
  releaseFirst(); await toolsCommitted;
  assert(![...records.values()].some(record => record.kind === 'tool'));
  await runtime.submit('Correction during work', { requestId: 'ordered-steer' }); await runtime.queue;
  const archive = [...records.values()];
  const steer = archive.findIndex(record => record.key === 'user:ordered-steer');
  assert(archive.findIndex(record => record.kind === 'tool') < steer);
  assert(archive.findIndex(record => record.kind === 'echo') < steer);
  assert.equal(calls.filter(call => call === 'ready').length, 1);
  releaseFinal(); await idle(runtime);
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  assert.equal(entries.items.filter(entry => entry.kind === 'pi.reset').length, 1);
});

test('human clarification is durable, host answered, one-use and cancelled on stop', async t => {
  const { runtime, events } = await fixture(t, [
    fauxAssistantMessage(fauxToolCall('human_request', { prompt: 'Which folder?' }), { stopReason: 'toolUse' }),
    fauxAssistantMessage('Waiting for your answer'),
  ]);
  await runtime.submit('Organize this', { requestId: 'clarify' }); await idle(runtime);
  const request = events.find(event => event.type === 'human_request').request;
  assert.equal(request.status, 'pending');
  await runtime.respondHuman(request.id, 'Downloads');
  const state = await runtime.harness.snapshot(runtime.HumanState, runtime.id, runtime.context);
  assert.equal(state.requests[request.id].response, 'Downloads');
  assert.equal(state.requests[request.id].status, 'completed');
  await assert.rejects(runtime.respondHuman(request.id, 'Pictures'), /unavailable/);
  await runtime.conversation.commit(async tx => { (await tx.doc(runtime.HumanState, runtime.id)).requests.test = { id: 'test', status: 'pending', expiresAt: Date.now() + 10000 }; }, runtime.context);
  await runtime.stop();
  const stopped = await runtime.harness.snapshot(runtime.HumanState, runtime.id, runtime.context);
  assert.equal(stopped.requests.test.status, 'cancelled');
});
