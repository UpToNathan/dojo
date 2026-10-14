import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { PiRuntime } from './pi-runtime.mjs';

const schema = { type: 'object', additionalProperties: true };
const tools = ['desktop_observe', 'desktop_decode_model', 'desktop_screenshot', 'desktop_capture_action', 'desktop_action', 'desktop_release'].map(name => ({ name, inputSchema: schema }));
const image = { type: 'image', data: 'fixture-png', mimeType: 'image/png' };
const toolRound = calls => fauxAssistantMessage(calls.map(([name, args]) => fauxToolCall(name, args)), { stopReason: 'toolUse' });

async function fixture(t, responses, handler, extraConfig = {}) {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-desktop-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider); faux.setResponses(responses);
  const calls = [], clients = [], records = new Map();
  const open = async () => {
    const client = { tools: extraConfig.desktopTools ?? tools, closed: false, pending: new Set(), close() {
      this.closed = true;
      for (const reject of this.pending) reject(new Error('desktop disconnected'));
      this.pending.clear();
    }, async call(_method, { name, arguments: args }) {
      calls.push({ name, args });
      return await handler(name, args, this);
    } };
    clients.push(client); return client;
  };
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: {
    async append(record) { records.set(record.key, record); }, async readyView() { return '<chat>\n</chat>'; },
    async zoom() { return 'original'; }, async date() { return '2026-01-01'; },
  }, config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'test', instructions: 'fixed', browserTools: false, desktopClientFactory: open, ...extraConfig } });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  return { runtime, calls, clients, records };
}
async function settled(runtime) {
  await runtime.queue;
  await runtime.conversation.waitForIdle(runtime.context);
  await runtime.archived;
}
const observe = id => ({ content: [{ type: 'text', text: JSON.stringify({ capture_id: id, actionable: true }) }, image] });

for (const outcome of ['completed', 'failed']) test(`${outcome} task resets its cursor without closing the desktop connection`, async t => {
  const { runtime, calls, clients } = await fixture(t, [
    toolRound([['desktop_action', { action: 'move', output: 'fixture', x: 10, y: 10 }]]),
    outcome === 'completed' ? fauxAssistantMessage('Done') : fauxAssistantMessage('', { stopReason: 'error', errorMessage: 'fixture failure' }),
  ], async () => ({ content: [{ type: 'text', text: '{"handled":true}' }] }));
  await runtime.submit('Move the cursor');
  await settled(runtime);
  assert(calls.some(call => call.name === 'desktop_action' && call.args.action === 'reset'));
  assert.equal(clients[0].closed, false);
});

test('failed cursor reset disconnects its seat instead of leaving it visible', async t => {
  const { runtime, clients } = await fixture(t, [
    toolRound([['desktop_action', { action: 'move', output: 'fixture', x: 10, y: 10 }]]),
    fauxAssistantMessage('Done'),
  ], async (_name, args) => ({ isError: args.action === 'reset', content: [] }));
  await runtime.submit('Move the cursor');
  await settled(runtime);
  assert.equal(clients[0].closed, true);
  assert.equal(runtime.desktop, undefined);
});

test('same-model foreground decoding stays scoped across eight concurrent summary jobs', async t => {
  let release, allReady, ready = 0, foregroundRound = 0;
  const gate = new Promise(resolve => { release = resolve; });
  const reached = new Promise(resolve => { allReady = resolve; });
  const modelIds = new Set();
  const dispatch = async (context, _options, _state, model) => {
    modelIds.add(model.id);
    const prompt = context.messages.map(message => typeof message.content === 'string' ? message.content : JSON.stringify(message.content)).join('\n');
    const summary = /summary-fixture-(\d+)/.exec(prompt);
    if (summary) {
      if (++ready === 9) allReady();
      await gate;
      return fauxAssistantMessage(`summary ${summary[1]}`);
    }
    if (foregroundRound++ === 0) return toolRound([['desktop_observe', {}]]);
    if (foregroundRound === 2) {
      if (++ready === 9) allReady();
      await gate;
      return fauxAssistantMessage('click(x=0,y=2)');
    }
    return fauxAssistantMessage('finished()');
  };
  const { runtime, calls } = await fixture(t, Array(20).fill(dispatch), async (name, args) => {
    if (name === 'desktop_observe') return observe('one');
    if (name === 'desktop_decode_model') {
      assert(['click(x=0,y=2)', 'finished()'].includes(args.raw), 'summary must never reach decoder');
      return { content: [{ type: 'text', text: JSON.stringify({ capture_id: args.capture_id, image_width: 100, image_height: 100,
        outcome: args.raw.startsWith('click') ? { Action: { Click: { at: { delivered_x: 0, delivered_y: 2 }, button: 'Left', count: 1 } } } : { Done: { text: 'finished' } } }) }] };
    }
    return { content: [{ type: 'text', text: '{"effect":"handled","post_observation":{"capture_id":"two"}}' }, image] };
  }, { desktopModelProfile: 'ui_tars' });
  t.after(() => release());
  await runtime.submit('Run foreground', { requestId: 'concurrent-scope' });
  const summaries = Array.from({ length: 8 }, (_, i) => runtime.summarize(`summary-fixture-${i}`));
  const timer = setTimeout(allReady, 5000);
  await reached; clearTimeout(timer);
  assert.equal(ready, 9, 'foreground request and eight summaries overlap');
  assert.equal(runtime.summaryJobs.size, 8);
  release();
  assert.deepEqual(await Promise.all(summaries), Array.from({ length: 8 }, (_, i) => `summary ${i}`));
  await settled(runtime);
  assert.deepEqual([...modelIds], ['test']);
  assert.deepEqual(calls.map(call => call.name), ['desktop_observe', 'desktop_decode_model', 'desktop_capture_action', 'desktop_decode_model', 'desktop_action']);
  assert.equal((await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context)).toolAttempts, 4);
});


const action = () => ({ content: [{ type: 'text', text: JSON.stringify({ effect: 'handled', handled: true, verified: false, requires_fresh_observation: true }) }] });

test('Pi commits fresh observation before ordered action and refuses reuse in the same round', async t => {
  const { runtime, calls, records } = await fixture(t, [
    toolRound([
      ['desktop_observe', { output: 'test', window_id: 42 }],
      ['desktop_capture_action', { capture_id: 'capture-one', x: 0, y: 0, input: { kind: 'click', button: 'left' } }],
      ['desktop_capture_action', { capture_id: 'capture-one', x: 1, y: 1, input: { kind: 'move' } }],
    ]),
    fauxAssistantMessage('Observed and acted'),
  ], async name => name === 'desktop_observe' ? observe('capture-one') : action());
  await runtime.submit('Run', { requestId: 'ordered' }); await settled(runtime);
  assert.deepEqual(calls.map(x => x.name), ['desktop_observe', 'desktop_capture_action', 'desktop_action']);
  const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
  assert.equal(state.recoveryRequired, true); assert.equal(state.captureId, null);
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  const results = entries.items.filter(entry => entry.kind === 'pi.tool-result');
  assert.equal(results.length, 3);
  assert(results.some(entry => JSON.stringify(entry).includes('Desktop recovery required')));
  assert(results.some(entry => JSON.stringify(entry).includes('fixture-png')));
  assert([...records.values()].some(record => record.kind === 'echo' && record.text.includes('fixture-png')));
});

test('Unknown receipt and legacy mutation each require a new observation', async t => {
  const { runtime, calls } = await fixture(t, [
    toolRound([['desktop_observe', {}]]),
    toolRound([['desktop_capture_action', { capture_id: 'one' }]]),
    toolRound([['desktop_capture_action', { capture_id: 'one' }], ['desktop_observe', {}]]),
    toolRound([['desktop_action', { action: 'reset' }], ['desktop_capture_action', { capture_id: 'two' }]]),
    fauxAssistantMessage('Stopped safely'),
  ], async name => name === 'desktop_observe'
    ? observe(calls.filter(call => call.name === 'desktop_observe').length === 1 ? 'one' : 'two')
    : { content: [{ type: 'text', text: '{"effect":"unknown","requires_fresh_observation":true}' }] });
  await runtime.submit('Run', { requestId: 'unknown' }); await settled(runtime);
  assert.deepEqual(calls.map(x => x.name), ['desktop_observe', 'desktop_capture_action', 'desktop_observe', 'desktop_action', 'desktop_action']);
});

test('Stop disconnects blocked desktop work and next run reopens with no old capture', { timeout: 5000 }, async t => {
  let began;
  const started = new Promise(resolve => { began = resolve; });
  const { runtime, calls, clients } = await fixture(t, [
    toolRound([['desktop_observe', {}]]),
    toolRound([['desktop_capture_action', { capture_id: 'old' }]]),
  ], async (name, _args, client) => {
    if (name === 'desktop_observe') return observe('old');
    began();
    return new Promise((_resolve, reject) => client.pending.add(reject));
  });
  await runtime.submit('Run', { requestId: 'blocked' }); await started;
  await runtime.stop();
  assert.equal(clients[0].closed, true);
  assert.equal(runtime.desktop, undefined);
  assert.equal(calls.filter(call => call.name === 'desktop_capture_action').length, 1);
  const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
  assert.equal(state.recoveryRequired, true);
  assert.equal(state.captureId, null);
});


test('Pi persists tool attempt budget before sending and refuses the next effect', async t => {
  const { runtime, calls } = await fixture(t, [
    toolRound([['desktop_observe', {}], ['desktop_capture_action', { capture_id: 'one' }]]),
    fauxAssistantMessage('Budget reached'),
  ], async () => observe('one'), { desktopMaxToolAttempts: 1 });
  await runtime.submit('Run', { requestId: 'budget' }); await settled(runtime);
  assert.deepEqual(calls.map(call => call.name), ['desktop_observe', 'desktop_action']);
  const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
  assert.equal(state.toolAttempts, 1);
  assert.equal(typeof state.catalog, 'string');
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  assert(entries.items.some(entry => JSON.stringify(entry).includes('Desktop tool budget exhausted')));
});

test('Pi accepts only the fresh post-effect capture for the next attempt', async t => {
  let mutations = 0;
  const { runtime, calls } = await fixture(t, [
    toolRound([['desktop_observe', {}], ['desktop_capture_action', { capture_id: 'one', observe_after: true }], ['desktop_capture_action', { capture_id: 'two' }], ['desktop_capture_action', { capture_id: 'one' }]]),
    fauxAssistantMessage('Finished'),
  ], async name => {
    if (name === 'desktop_observe') return observe('one');
    if (name === 'desktop_action') return action();
    mutations++;
    return mutations === 1 ? { content: [{ type: 'text', text: JSON.stringify({ effect: 'handled', post_observation: { capture_id: 'two' } }) }, image] } : action();
  });
  await runtime.submit('Run', { requestId: 'post-effect' }); await settled(runtime);
  assert.deepEqual(calls.map(call => call.name), ['desktop_observe', 'desktop_capture_action', 'desktop_capture_action', 'desktop_action']);
  assert.equal(mutations, 2);
});

test('text profile goes through Rust decode before Pi admits exactly one action', async t => {
  const { runtime, calls } = await fixture(t, [
    toolRound([['desktop_observe', {}]]),
    fauxAssistantMessage("click(x=0,y=2)"),
    fauxAssistantMessage("finished()"),
  ], async (name, args) => {
    if (name === 'desktop_observe') return observe('one');
    if (name === 'desktop_decode_model') return { content: [{ type: 'text', text: JSON.stringify({
      capture_id: args.capture_id, image_width: 100, image_height: 100,
      outcome: args.raw.startsWith('click') ? { Action: { Click: { at: { delivered_x: 0, delivered_y: 2 }, button: 'Left', count: 1 } } } : { Done: { text: 'finished' } },
    }) }] };
    return { content: [{ type: 'text', text: JSON.stringify({ effect: 'handled', post_observation: { capture_id: 'two' } }) }, image] };
  }, { desktopModelProfile: 'ui_tars' });
  await runtime.submit('Run', { requestId: 'text-profile' }); await settled(runtime);
  assert.deepEqual(calls.map(call => call.name), ['desktop_observe', 'desktop_decode_model', 'desktop_capture_action', 'desktop_decode_model', 'desktop_action']);
  const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
  assert.equal(state.toolAttempts, 4);
  const executed = calls.find(call => call.name === 'desktop_capture_action');
  assert.equal(executed.args.x, 0); assert.equal(executed.args.capture_id, 'one');
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  assert(entries.items.some(entry => entry.kind === 'pi.assistant' && entry.model?.[0]?.content.some(block => block.type === 'toolCall' && block.name === 'desktop_capture_action')));
});




test('SQLite reopen preserves unresolved external effects and prevents native follow-ons', async t => {
  const { runtime, calls } = await fixture(t, [
    toolRound([['desktop_observe', {}], ['desktop_capture_action', { capture_id: 'new' }]]),
    fauxAssistantMessage('Blocked'),
  ], async name => name === 'desktop_observe' ? observe('new') : { content: [{ type: 'text', text: '{"effect":"unknown","accepted_bytes":83}' }] }, { storage: undefined });
  await runtime.conversation.commit(async tx => { (await tx.doc(runtime.DesktopState, runtime.id)).externalUnknown = true; }, runtime.context);
  await runtime.close();
  const reopened = await PiRuntime.open({ stateDir: runtime.stateDir, root: runtime.root, memory: runtime.memory, config: runtime.config });
  t.after(() => reopened.close());
  assert.equal((await reopened.harness.snapshot(reopened.DesktopState, reopened.id, reopened.context)).externalUnknown, true);
  await reopened.submit('Continue', { requestId: 'continue-external' }); await settled(reopened);
  assert.equal(calls.filter(call => call.name === 'desktop_capture_action').length, 0);
});

 test('an unbound compatibility image retires the prior grounding capture', async t => {
  const {runtime,calls}=await fixture(t,[toolRound([
    ['desktop_observe',{}],['desktop_screenshot',{}],['desktop_capture_action',{capture_id:'one'}]
  ]),fauxAssistantMessage('finished')],async name=>name==='desktop_observe'?observe('one'):{content:[image]});
  await runtime.submit('Run',{requestId:'unbound-image'});await settled(runtime);
  assert.deepEqual(calls.map(call=>call.name),['desktop_observe','desktop_screenshot', 'desktop_action']);
  const state=await runtime.harness.snapshot(runtime.DesktopState,runtime.id,runtime.context);
  assert.equal(state.captureId,null);assert.equal(state.recoveryRequired,true);
});


test('Pi discovers original Driver schemas and relays results without changing images', async t => {
  const originalSchema = { type: 'object', properties: { capture_id: { type: 'string' } }, required: ['capture_id'] };
  const { runtime, calls } = await fixture(t, [toolRound([['driver_click', { capture_id: 'one' }]]), 'done'], async (name, args) => {
    if (name === 'desktop_action' && args.action === 'reset') return { content: [{ type: 'text', text: '{"handled":true}' }] };
    if (name === 'desktop_driver_tools') return { content: [{ type: 'text', text: JSON.stringify({ tools: [{ name: 'click', description: 'original', inputSchema: originalSchema }] }) }] };
    assert.equal(name, 'desktop_driver_call');
    assert.deepEqual(args, { name: 'click', arguments: { capture_id: 'one' } });
    return { content: [image, { type: 'text', text: 'original result' }] };
  }, { desktopTools: [...tools, { name: 'desktop_driver_tools', inputSchema: schema }, { name: 'desktop_driver_call', inputSchema: schema }] });
  await runtime.submit('click', { requestId: 'driver-click' }); await settled(runtime);
  assert.equal(calls.filter(call => call.name === 'desktop_driver_call').length, 1);
});
