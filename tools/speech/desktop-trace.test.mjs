import test from 'node:test';
import assert from 'node:assert/strict';
import { desktopTraceEvents, desktopTraceUsage } from './desktop-trace.mjs';
import { helperBinary } from './helper-binary.mjs';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { PiRuntime } from './pi-runtime.mjs';

test('trace projection binds Pi calls and receipts without claiming delivery proves success', () => {
  const events = desktopTraceEvents([
    { id: 1, kind: 'pi.assistant', model: [{ content: [
      { type: 'toolCall', id: 'a', name: 'desktop_capture_action', arguments: { capture_id: 'c', x: 0 } },
      { type: 'toolCall', id: 'b', name: 'read', arguments: {} },
    ] }] },
    { id: 2, kind: 'pi.tool-result', model: [{ toolCallId: 'a', content: [{ type: 'text', text: '{"effect":"handled"}' }, { type: 'image', data: 'private pixels' }] }] },
  ]);
  assert.equal(events.length, 2);
  assert.equal(events[0].observation_id, 'c');
  assert.equal(events[1].call_id, 'a');
  assert.equal(events[1].pi_intent_id, events[0].pi_intent_id);
  assert.equal(events[1].effect, 'unknown');
  assert(!JSON.stringify(events).includes('private pixels'));
});

test('trace includes the installed native, perception, Driver and browser routes', () => {
  const names = ['driver_click', 'browser_control_execute', 'parse_visual_regions', 'list_windows', 'list_apps', 'get_screen_size'];
  const entries = names.flatMap((name, index) => [
    { id: `intent-${index}`, kind: 'pi.assistant', model: [{ content: [{ type: 'toolCall', id: `call-${index}`, name, arguments: {} }] }] },
    { id: `receipt-${index}`, kind: 'pi.tool-result', model: [{ toolName: name, toolCallId: `call-${index}`, content: [{ type: 'text', text: 'delivered' }] }] },
  ]);
  const events = desktopTraceEvents(entries);
  assert.deepEqual(events.filter(event => event.kind === 'intent').map(event => event.route), names);
  for (let index = 0; index < names.length; index++) {
    assert.equal(events[index * 2 + 1].pi_intent_id, `intent-${index}`);
    assert.deepEqual(events[index * 2 + 1].data, { result: { text: 'delivered' }, is_error: false });
  }
});

test('trace preserves unknown effects and plain error evidence', () => {
  const receipt = (id, text) => ({ id, kind: 'pi.tool-result', model: [{ toolName: 'browser_control_execute', toolCallId: id, isError: true, content: [{ type: 'text', text }] }] });
  const events = desktopTraceEvents([
    receipt('timeout', 'Command timed out after submission'),
    receipt('unknown', '{"effect":"refused","unknownCommandIds":[9]}'),
    receipt('partial', '{"effect":"partial"}'),
    receipt('refused', '{"effect":"refused"}'),
  ]);
  assert.deepEqual(events.map(event => event.effect), ['unknown', 'unknown', 'partial', 'refused']);
  assert.equal(events[0].data.result.text, 'Command timed out after submission');
  assert(events.every(event => event.data.is_error));
});

test('verification evidence is separate from effect delivery', () => {
  const events = desktopTraceEvents([
    { id: 'intent', kind: 'pi.assistant', model: [{ content: [{ type: 'toolCall', id: 'verify', name: 'desktop_verify', arguments: {} }] }] },
    { id: 'receipt', kind: 'pi.tool-result', model: [{ toolCallId: 'verify', content: [{ type: 'text', text: '{"status":"satisfied","predicates":[]}' }] }] },
  ]);
  assert.deepEqual(events.map(event => event.kind), ['intent', 'receipt', 'verification']);
  assert.equal(events[1].effect, 'unknown');
  assert.equal(events[2].effect, null);
  assert.equal(events[2].data.status, 'satisfied');
});

test('Rust analysis pairs exported receipts with their canonical Pi intent', () => {
  const events = desktopTraceEvents([
    { id: 'assistant-entry', kind: 'pi.assistant', model: [{ content: [
      { type: 'toolCall', id: 'finished', name: 'driver_click', arguments: {} },
      { type: 'toolCall', id: 'interrupted', name: 'browser_control_execute', arguments: {} },
    ] }] },
    { id: 'different-result-entry', kind: 'pi.tool-result', model: [{ toolCallId: 'finished', content: [{ type: 'text', text: '{"effect":"handled"}' }] }] },
  ]);
  const codec = input => JSON.parse(execFileSync(helperBinary(process.cwd(), 'dojo-computer-recording'), [], { input: JSON.stringify(input), encoding: 'utf8' }));
  const trace = codec({ command: 'record', session_id: 'fixture', events });
  const report = codec({ command: 'analyze', trace });
  assert.deepEqual(report.invalid_receipts, []);
  assert.deepEqual(report.unresolved_intents, ['assistant-entry']);
  assert.equal(report.unknown, 1);
  assert.equal(report.safe_to_replay_effects, false);
});

test('trace projection bounds desktop event count', () => {
  const entry = { id: 1, kind: 'pi.assistant', model: [{ content: [{ type: 'toolCall', id: 'a', name: 'desktop_observe', arguments: {} }] }] };
  assert.throws(() => desktopTraceEvents(Array(1001).fill(entry)), /1000 events/);
});

test('canonical usage follows exclusive pi-ai input semantics and preserves absent or invalid costs', () => {
  const entry = (id, cost) => ({ id, kind: 'pi.assistant', model: [{ usage: {
    input: 10, output: 4, cacheRead: 3, cacheWrite: 2, ...(cost === undefined ? {} : { cost: { total: cost } }),
  } }] });
  const samples = desktopTraceUsage([entry('provider-private', 0.000125), entry(2), entry(3, NaN), entry(4, -1), entry(5, 1e99), entry(6, 0)], 'conversation-a');
  assert.equal(samples[0].input_tokens, 15);
  assert.equal(samples[0].cost_microusd, 125);
  assert.deepEqual(samples.slice(1, 5).map(s => s.cost_microusd), [null, null, null, null]);
  assert.equal(samples[5].cost_microusd, 0);
  assert.match(samples[0].receipt_digest, /^[a-f0-9]{64}$/);
  assert(!JSON.stringify(samples).includes('provider-private'));
  assert.notEqual(samples[0].receipt_digest, desktopTraceUsage([entry('provider-private', 0)], 'conversation-b')[0].receipt_digest);
  assert.equal(samples[0].receipt_digest, desktopTraceUsage([entry('provider-private', 0)], 'conversation-a')[0].receipt_digest);
  const invalid = entry(7); invalid.model[0].usage.input = 0.5;
  assert.throws(() => desktopTraceUsage([invalid], 'a'), /token usage/);
});

test('runtime exports actual canonical assistant usage through Rust without mutating history', async t => {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-usage-pi-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider);
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(),
    memory: { async append() {}, async readyView() { return '<chat></chat>'; }, async zoom() { return ''; }, async date() { return '2026-01-01'; } },
    config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'test', instructions: 'fixed', browserTools: false, desktopTools: false } });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  const first = fauxAssistantMessage([fauxToolCall('date', { id: 0 })], { stopReason: 'toolUse', responseId: 'private-provider-id' });
  first.usage = { input: 10, output: 4, cacheRead: 3, cacheWrite: 2, totalTokens: 19, cost: { total: 0.000125 } };
  const second = fauxAssistantMessage('finished');
  second.usage = { input: 7, output: 2, cacheRead: 0, cacheWrite: 0, totalTokens: 9 };
  // The faux provider estimates/overwrites supplied usage. Commit fixture provider
  // receipts using Pi's actual immutable transaction API instead.
  await runtime.conversation.commit(async tx => {
    await tx.appendEntry(runtime.id, { kind: 'pi.assistant', model: [first] });
    await tx.appendEntry(runtime.id, { kind: 'pi.assistant', model: [second] });
  }, runtime.context);
  const before = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  const trace = await runtime.exportDesktopTrace();
  assert.equal(trace.schema_version, 'dojo.computer.trace/1');
  assert.deepEqual(trace.usage, { samples: 2, input_tokens: 22, output_tokens: 6, total_tokens: 28,
    cache_read_tokens: 3, cache_write_tokens: 2, known_cost_microusd: 125, samples_without_cost: 1 });
  assert(!JSON.stringify(trace).includes('private-provider-id'));
  assert.deepEqual(await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context), before);
});

test('step effects retain their exact nested capture identity in exported intents', () => {
  const events=desktopTraceEvents([{id:7,kind:'pi.assistant',model:[{content:[{type:'toolCall',id:'step',name:'computer_steps_apply',arguments:{run_id:'r',action:{capture_id:'owned',x:0}}}]}]}]);
  assert.equal(events.length,1);assert.equal(events[0].route,'computer_steps_apply');
  assert.equal(events[0].observation_id,'owned');assert.equal(events[0].data.arguments.action.x,0);
});
