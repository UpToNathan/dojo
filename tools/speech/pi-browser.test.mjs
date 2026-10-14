import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { MemoryStorage } from '@earendil-works/pi-durable';
import { PiRuntime } from './pi-runtime.mjs';

const result = value => ({ content: [{ type: 'text', text: JSON.stringify(value) }] });
const execute = args => fauxAssistantMessage([fauxToolCall('browser_control_execute', args)], { stopReason: 'toolUse' });
async function fixture(t, responses, handler = () => result({ value: 'ok' }), extraConfig = {}) {
  const stateDir = await mkdtemp(join(tmpdir(), 'dojo-pi-browser-'));
  const faux = fauxProvider({ models: [{ id: 'test', contextWindow: 200000 }] });
  const models = createModels(); models.setProvider(faux.provider); faux.setResponses(responses);
  const calls = [];
  let generation = 0;
  const mcp = { tools: ['execute', 'execute_cancel', 'session_resume', 'status', 'skill'].map(name => ({ name, inputSchema: { type: 'object', additionalProperties: true } })), closed: false,
    close() { this.closed = true; }, async call(_method, { name, arguments: args }, options) {
      calls.push({ name, args });
      if (name === 'session_resume') return result({ generation: ++generation });
      return handler(name, args, options);
    } };
  const runtime = await PiRuntime.open({ stateDir, root: process.cwd(), memory: { async append() {}, async readyView() { return '<chat>\n</chat>'; } },
    config: { storage: new MemoryStorage(), models, provider: 'faux', model: 'test', instructions: 'fixed', desktopTools: false, browserClientFactory: async () => mcp, ...extraConfig } });
  t.after(async () => { await runtime.close(); await rm(stateDir, { recursive: true, force: true }); });
  return { runtime, calls, mcp };
}
async function settled(runtime) { await runtime.queue; await runtime.conversation.waitForIdle(runtime.context); await runtime.archived; }
const retired = () => result({ nativeRetired: true, extensionAcknowledged: true, unknownCommandIds: [] });

test('each Pi task resumes a fresh generation and retires it while preserving the MCP connection', async t => {
  const { runtime, calls, mcp } = await fixture(t, [execute({ code: 'return page.url()' }), fauxAssistantMessage('done'), execute({ code: 'return page.title()' }), fauxAssistantMessage('done')], name => name === 'execute_cancel' ? retired() : result({ url: 'https://example.invalid' }));
  await runtime.submit('Read URL'); await settled(runtime);
  await runtime.submit('Read title'); await settled(runtime);
  assert.deepEqual(calls.map(call => call.name), ['session_resume', 'execute', 'execute_cancel', 'session_resume', 'execute', 'execute_cancel']);
  assert.equal(mcp.closed, false);
  assert.equal(calls.some(call => ['session_reset', 'session_delete'].includes(call.name)), false);
});

test('Pi cancellation reaches the browser while execute is still waiting', async t => {
  let started, release;
  const pending = new Promise(resolve => { started = resolve; });
  const { runtime, calls } = await fixture(t, [execute({ code: 'await page.waitForTimeout(60000)' })], name => {
    if (name === 'execute') { started(); return new Promise(resolve => { release = resolve; }); }
    if (name === 'execute_cancel') { release(result({ cancelled: true })); return retired(); }
    return result({});
  });
  await runtime.submit('Wait'); await pending;
  await runtime.stop();
  assert.deepEqual(calls.slice(0, 3).map(call => call.name), ['session_resume', 'execute', 'execute_cancel']);
});

test('uncertain admitted commands block later execution without an automatic retry', async t => {
  const { runtime, calls } = await fixture(t, [execute({ code: 'await page.click("button")' }), fauxAssistantMessage('done'), execute({ code: 'await page.click("button")' }), fauxAssistantMessage('stopped')], name => name === 'execute_cancel' ? result({ nativeRetired: true, unknownCommandIds: [7] }) : result({}));
  await runtime.submit('Click'); await settled(runtime);
  await runtime.submit('Continue'); await settled(runtime);
  assert.equal(calls.filter(call => call.name === 'execute').length, 1);
  assert.equal((await runtime.harness.snapshot(runtime.BrowserState, runtime.id, runtime.context)).uncertain, true);
});

test('Pi refuses an explicit foreign browser session before transport', async t => {
  const { runtime, calls } = await fixture(t, [execute({ code: 'return page.url()', session: 'other-agent' }), fauxAssistantMessage('stopped')]);
  await runtime.submit('Inspect'); await settled(runtime);
  assert.deepEqual(calls, []);
});

test('structured unknown execute results survive cleanup and block the next task', async t => {
  const { runtime, calls } = await fixture(t, [execute({ code: 'await page.click("button")' }), fauxAssistantMessage('uncertain'), execute({ code: 'await page.click("button")' }), fauxAssistantMessage('blocked')], name => name === 'execute_cancel' ? retired() : { isError: true, ...result({ effect: 'unknown', unknownCommandIds: [9] }) });
  await runtime.submit('Click'); await settled(runtime);
  await runtime.submit('Try again'); await settled(runtime);
  assert.equal(calls.filter(call => call.name === 'execute').length, 1);
  assert.equal((await runtime.harness.snapshot(runtime.BrowserState, runtime.id, runtime.context)).uncertain, true);
});

test('browser status reads never resume or retire a browser generation', async t => {
  const status = fauxAssistantMessage([fauxToolCall('browser_control_status', {})], { stopReason: 'toolUse' });
  const { runtime, calls } = await fixture(t, [status, fauxAssistantMessage('ready')]);
  await runtime.submit('Check status'); await settled(runtime);
  assert.deepEqual(calls.map(call => call.name), ['status']);
});

test('Pi stop retires then cancels its RPC without waiting for a remote script to finish', async t => {
  let begin;
  const started = new Promise(resolve => { begin = resolve; });
  const { runtime, calls, mcp } = await fixture(t, [execute({ code: 'await new Promise(() => {})' })], (name, _args, options) => {
    if (name === 'execute_cancel') return retired();
    if (name === 'execute') {
      begin();
      return new Promise((_resolve, reject) => options.signal.addEventListener('abort', () => reject(new Error('request aborted')), { once: true }));
    }
    return result({});
  });
  await runtime.submit('Wait'); await started;
  await Promise.race([runtime.stop(), new Promise((_resolve, reject) => { const timer = setTimeout(() => reject(new Error('Pi stop waited for remote execution')), 1000); timer.unref(); })]);
  assert.equal(mcp.closed, false);
  assert.equal(calls.at(-1).name, 'execute_cancel');
  assert.equal((await runtime.harness.snapshot(runtime.BrowserState, runtime.id, runtime.context)).uncertain, false);
});

test('an unknown browser outcome cannot be bypassed with native desktop input', async t => {
  const native = [];
  const desktop = { tools: ['desktop_action', 'desktop_observe', 'desktop_capture_action'].map(name => ({ name, inputSchema: { type: 'object', additionalProperties: true } })), close() {}, async call(_method, { name, arguments: args }) { native.push({ name, args }); return result({ effect: 'handled' }); } };
  const round = fauxAssistantMessage([fauxToolCall('browser_control_execute', { code: 'await page.click("button")' }), fauxToolCall('desktop_action', { action: 'click', button: 'left' })], { stopReason: 'toolUse' });
  const { runtime } = await fixture(t, [round, fauxAssistantMessage('blocked')], name => name === 'execute_cancel' ? retired() : { isError: true, ...result({ effect: 'unknown', unknownCommandIds: [9] }) }, { desktopTools: true, desktopClientFactory: async () => desktop });
  await runtime.submit('Click'); await settled(runtime);
  assert(native.every(call => call.name === 'desktop_action' && call.args.action === 'reset'));
});
