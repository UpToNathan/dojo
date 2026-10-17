import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { existsSync } from 'node:fs';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { PiRuntime } from './pi-runtime.mjs';
import { computerStepPolicy, nestDesktopSchema } from './computer-steps.mjs';
import { DesktopMcp } from './desktop-extension.mjs';
import { helperBinary } from './helper-binary.mjs';

// The installed or release helper is the one Dojo runs; the crate's own debug build is the fallback.
const crateBinary = join(process.cwd(), 'crates/dojo-computer-verification/target/debug/dojo-computer-verification');
const installed = helperBinary(process.cwd(), 'dojo-computer-verification');
const policyBinary = existsSync(installed) ? installed : crateBinary;
const schema = { type: 'object', additionalProperties: true };
const tools = ['desktop_observe', 'desktop_capture_action', 'desktop_release'].map(name => ({ name, inputSchema: schema }));
const image = { type: 'image', data: 'step-fixture-png', mimeType: 'image/png' };
const round = calls => fauxAssistantMessage(calls.map(([name, args]) => fauxToolCall(name, args)), { stopReason: 'toolUse' });
async function settled(runtime) { await runtime.queue; await runtime.conversation.waitForIdle(runtime.context); await runtime.archived; }
async function fixture(t, handler, sqlite = false, catalog = tools) {
  assert(existsSync(policyBinary), 'Build the real Rust step codec with cargo build --manifest-path crates/dojo-computer-verification/Cargo.toml');
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-step-pi-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider);
  const calls = [], clients = [];
  const open = async () => {
    const client = { tools: catalog, closed: false, pending: new Set(), close() { this.closed = true; for (const reject of this.pending) reject(new Error('desktop disconnected')); this.pending.clear(); },
      async call(_method, { name, arguments: args }) { if (name === 'desktop_action' && args.action === 'reset') return effect('handled'); calls.push({ name, args }); return handler(name, args, this); } };
    clients.push(client); return client;
  };
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { async append() {}, async readyView() { return '<chat></chat>'; }, async zoom() { return ''; }, async date() { return '2026-01-01'; } },
    config: { ...(sqlite ? {} : { storage: new MemoryStorage() }), models, provider: 'faux', model: 'test', instructions: 'fixed', browserTools: false, desktopClientFactory: open, computerPolicyBinary: policyBinary } });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  const submit = async calls => { faux.setResponses([round(calls), fauxAssistantMessage('finished')]); await runtime.submit('One bounded step', { requestId: crypto.randomUUID() }); await settled(runtime); };
  await submit([['computer_steps_begin', { max_steps: 2, timeout_ms: 10000 }]]);
  const run = (await runtime.harness.snapshot(runtime.ComputerSteps, runtime.id, runtime.context)).run;
  assert.equal(run.phase.kind, 'observe');
  return { runtime, faux, calls, clients, submit, runId: run.run_id };
}
const observation = id => ({ content: [{ type: 'text', text: JSON.stringify({ capture_id: id, actionable: true }) }, image] });
const effect = outcome => ({ content: [{ type: 'text', text: JSON.stringify({ effect: outcome, verified: false }) }] });

test('nested catalog schemas relocate local pointers and preserve literal constraints', () => {
  const original = { type: 'object', $defs: { Point: { type: 'integer', minimum: 0 } },
    properties: { x: { $ref: '#/$defs/Point' }, literal: { const: { $ref: 'literal data' } },
      nullable: { anyOf: [{ $ref: '#/$defs/Point' }, { type: 'null' }] } }, required: ['x'], additionalProperties: false };
  const nested = nestDesktopSchema(original, 'action');
  assert.equal(nested.properties.x.$ref, '#/properties/action/$defs/Point');
  assert.equal(nested.properties.nullable.anyOf[0].$ref, '#/properties/action/$defs/Point');
  assert.deepEqual(nested.properties.literal, original.properties.literal);
  assert.deepEqual(nested.$defs, original.$defs);
  assert.equal(original.properties.x.$ref, '#/$defs/Point');
  assert.throws(() => nestDesktopSchema({ $ref: 'https://example.invalid/schema' }, 'action'), /local pointer/);
  assert.throws(() => nestDesktopSchema({ $ref: '#/$defs/Missing' }, 'action'), /unavailable/);
});

test('actual Rust catalog nested refs validate region, drag and model provenance before effects', async t => {
  assert(existsSync(join(process.cwd(), 'target/debug/dojo-agent')), 'Build dojo-agent to read its actual MCP tools/list catalog');
  const mcp = await DesktopMcp.open(process.cwd());
  const catalog = mcp.tools.filter(tool => ['desktop_observe', 'desktop_capture_action', 'desktop_release'].includes(tool.name));
  mcp.close(); // Only initialize/tools/list ran; all effect calls use the fixture below.
  const actionSchema = catalog.find(tool => tool.name === 'desktop_capture_action').inputSchema;
  assert.equal(actionSchema.properties.input.$ref, '#/$defs/CaptureInput');
  assert(actionSchema.$defs.ModelSource.properties.profile.$ref);
  let observations = 0;
  const f = await fixture(t, async name => name === 'desktop_observe' ? observation(`catalog-cap-${++observations}`) : effect('handled'), false, catalog);
  const validObservation = { output: 'fixture-output', region: { x: 0, y: 0, width: 64, height: 64 }, window_id: 42, scale: null };
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: { ...validObservation, region: { ...validObservation.region, width: 'bad' } } }]]);
  assert.equal(f.calls.length, 0, 'invalid Region reference is rejected before backend');
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: validObservation }]]);
  assert.equal(f.calls.length, 1);
  const action = { capture_id: 'catalog-cap-1', x: 1, y: 2,
    input: { kind: 'drag', button: 'left', path: [{ x: 3, y: 4 }, { x: 5, y: 6 }] },
    observe_after: false, model_source: { profile: 'ui_tars', parser_version: 'fixture/1', raw: 'drag fixture', capture_digest: 'a'.repeat(64), call_id: null } };
  for (const invalid of [
    { ...action, input: { ...action.input, button: 'invalid' } },
    { ...action, input: { ...action.input, path: [{ x: 'bad', y: 4 }] } },
    { ...action, model_source: { ...action.model_source, profile: 'invalid' } },
    { ...action, model_source: { ...action.model_source, call_id: 123 } },
    { ...action, injected: true },
  ]) {
    await f.submit([['computer_steps_apply', { run_id: f.runId, action: invalid }]]);
    assert.equal(f.calls.length, 1, 'invalid nested action denied before effects');
    assert.equal((await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run.phase.kind, 'ready');
  }
  // Previous model tasks ended and retired their captures. Ground this action in the same task.
  const freshAction = { ...action, capture_id: 'catalog-cap-2' };
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: validObservation }], ['computer_steps_apply', { run_id: f.runId, action: freshAction }]]);
  assert.deepEqual(f.calls[2], { name: 'desktop_capture_action', args: freshAction });
  const run = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  assert.equal(run.completed_prefix.length, 1);
  assert.equal(run.phase.kind, 'observe');
});

// Uses actual Pi durable tools/documents and the compiled Rust phase codec.
test('Pi step flow requires new captures and persists the completed prefix without fabricated verification', async t => {
  let observations = 0;
  const f = await fixture(t, async name => name === 'desktop_observe' ? observation(`cap-${++observations}`) : effect('handled'));
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: {} }],
    ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'cap-1', x: 0, y: 0, input: { kind: 'click', button: 'left' } } }],
    ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'cap-1', input: { kind: 'move' } } }]]);
  assert.deepEqual(f.calls.map(call => call.name), ['desktop_observe', 'desktop_capture_action']);
  let run = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  assert.equal(run.phase.kind, 'observe'); assert.equal(run.completed_prefix.length, 1);
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: {} }], ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'cap-2', input: { kind: 'move' } } }]]);
  run = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  assert.equal(run.phase.kind, 'finished'); assert.equal(run.completed_prefix.length, 2);
  assert.equal(f.calls.filter(call => call.name === 'desktop_capture_action').length, 2);
  const entries = await f.runtime.conversation.entries({ order: 'ascending' }, 100, undefined, f.runtime.context);
  assert(entries.items.some(entry => JSON.stringify(entry).includes('step-fixture-png')));
  assert(entries.items.some(entry => JSON.stringify(entry).includes('task_verified')));
});

test('SQLite reopen retains unknown effect and prevents replay or a replacement run', async t => {
  const f = await fixture(t, async name => name === 'desktop_observe' ? observation('one') : effect('unknown'), true);
  await f.submit([['computer_steps_observe', { run_id: f.runId, observation: {} }], ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'one', input: { kind: 'click' } } }]]);
  let run = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  assert.equal(run.phase.kind, 'stopped'); assert.equal(run.phase.ambiguous_step.step, 0); const intent = run.phase.ambiguous_step.intent_id;
  await f.runtime.close();
  const reopened = await PiRuntime.open({ stateDir: f.runtime.stateDir, root: f.runtime.root, memory: f.runtime.memory, config: f.runtime.config });
  t.after(() => reopened.close());
  f.faux.setResponses([round([['computer_steps_status', { run_id: f.runId }], ['computer_steps_observe', { run_id: f.runId, observation: {} }],
    ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'one', input: { kind: 'click' } } }], ['computer_steps_begin', {}]]), fauxAssistantMessage('blocked')]);
  await reopened.submit('Continue after reopen', { requestId: 'reopened' }); await settled(reopened);
  run = (await reopened.harness.snapshot(reopened.ComputerSteps, reopened.id, reopened.context)).run;
  assert.equal(run.phase.ambiguous_step.intent_id, intent); assert.equal(f.calls.filter(call => call.name === 'desktop_capture_action').length, 1);
});

test('durable intent crash gap becomes ambiguous on real Rust restart without any dispatch', async t => {
  const f = await fixture(t, async () => { throw new Error('No desktop call expected'); }, true);
  let run = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  run = await computerStepPolicy(f.runtime, { command: 'steps_transition', state: run, event: { kind: 'observed', capture_id: 'retired', observation_id: 'retired' }, now_ms: Date.now() });
  run = await computerStepPolicy(f.runtime, { command: 'steps_transition', state: run, event: { kind: 'begin_effect', capture_id: 'retired', intent_id: 'crash-intent', action_digest: 'a'.repeat(64) }, now_ms: Date.now() });
  await f.runtime.conversation.commit(async tx => { (await tx.doc(f.runtime.ComputerSteps, f.runtime.id)).run = run; }, f.runtime.context);
  await f.runtime.close();
  const reopened = await PiRuntime.open({ stateDir: f.runtime.stateDir, root: f.runtime.root, memory: f.runtime.memory, config: f.runtime.config }); t.after(() => reopened.close());
  f.faux.setResponses([round([['computer_steps_status', { run_id: f.runId }], ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'retired' } }]]), fauxAssistantMessage('blocked')]);
  await reopened.submit('Recover', { requestId: 'intent-restart' }); await settled(reopened);
  const state = (await reopened.harness.snapshot(reopened.ComputerSteps, reopened.id, reopened.context)).run;
  assert.equal(state.phase.reason, 'unknown_effect'); assert.equal(state.phase.ambiguous_step.intent_id, 'crash-intent'); assert.equal(f.calls.length, 0);
});

test('runtime stop closes an in-flight effect and keeps ambiguity in the Pi document', async t => {
  let started;
  const entered = new Promise(resolve => { started = resolve; });
  const f = await fixture(t, async (name, _args, client) => {
    if (name === 'desktop_observe') return observation('one');
    started(); return new Promise((_resolve, reject) => client.pending.add(reject));
  });
  f.faux.setResponses([round([['computer_steps_observe', { run_id: f.runId, observation: {} }], ['computer_steps_apply', { run_id: f.runId, action: { capture_id: 'one', input: { kind: 'click' } } }]]), fauxAssistantMessage('stopped')]);
  await f.runtime.submit('Attempt effect', { requestId: 'stop-effect' }); await entered; await f.runtime.stop(); await settled(f.runtime);
  const state = (await f.runtime.harness.snapshot(f.runtime.ComputerSteps, f.runtime.id, f.runtime.context)).run;
  assert.equal(state.phase.kind, 'stopped'); assert(state.phase.ambiguous_step); assert.equal(f.clients[0].closed, true);
  assert.equal(f.calls.filter(call => call.name === 'desktop_capture_action').length, 1);
});
