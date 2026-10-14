// Called only by the isolated Rust desktop acceptance test. Uses scripted Pi responses,
// the real MCP subprocess and disposable SQLite state. No provider or credential calls.
import assert from 'node:assert/strict';
import { mkdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { createModels } from '@earendil-works/pi-ai/models';
import { fauxProvider, fauxAssistantMessage, fauxToolCall } from '@earendil-works/pi-ai/providers/faux';
import { DesktopMcp } from './desktop-extension.mjs';
import { PiRuntime } from './pi-runtime.mjs';

const [evidence, output, window] = process.argv.slice(2);
assert(evidence && output && Number.isSafeInteger(Number(window)));
const root = process.cwd();
const calls = [], records = new Map();
const faux = fauxProvider({ models: [{ id: 'live-fixture', contextWindow: 200000 }] });
const models = createModels(); models.setProvider(faux.provider);
const toolRound = calls => fauxAssistantMessage(calls.map(([name, args]) => fauxToolCall(name, args)), { stopReason: 'toolUse' });
faux.setResponses([toolRound([['desktop_observe', { output, window_id: Number(window), scale: 0.5 }]])]);
const open = async () => {
  const client = await DesktopMcp.open(root);
  const call = client.call.bind(client);
  client.call = async (method, params, options) => {
    if (method === 'tools/call') calls.push(params.name);
    const result = await call(method, params, options);
    if (method === 'tools/call' && params.name === 'desktop_observe') {
      assert(!result.isError, JSON.stringify(result));
      const observation = JSON.parse(result.content.find(block => block.type === 'text').text);
      assert.equal(observation.actionable, true);
      const action = { capture_id: observation.capture_id, x: observation.image_width * 0.5, y: observation.image_height * 0.5, input: { kind: 'click', button: 'left' } };
      faux.setResponses([toolRound([['desktop_capture_action', action], ['desktop_capture_action', action]]), fauxAssistantMessage('Capture acceptance finished')]);
    }
    return result;
  };
  return client;
};
await mkdir(join(evidence, 'pi'), { recursive: true });
const runtime = await PiRuntime.open({ stateDir: join(evidence, 'pi'), root, memory: {
  async append(record) { records.set(record.key, record); },
  async readyView() { return '<chat>\n</chat>'; }, async zoom() { return 'fixture'; }, async date() { return '2026-10-12'; },
}, config: { models, provider: 'faux', model: 'live-fixture', instructions: 'Isolated desktop acceptance fixture.', desktopClientFactory: open } });
try {
  await runtime.submit('Observe the isolated fixture and click once', { requestId: 'capture-live' });
  await runtime.queue;
  await runtime.conversation.waitForIdle(runtime.context);
  await runtime.archived;
  assert.deepEqual(calls, ['desktop_driver_tools', 'desktop_observe', 'desktop_capture_action', 'desktop_action']);
  const entries = await runtime.conversation.entries({ order: 'ascending' }, 100, undefined, runtime.context);
  const results = entries.items.filter(entry => entry.kind === 'pi.tool-result');
  assert.equal(results.length, 3);
  const actionResult = results.flatMap(entry => entry.model ?? []).find(entry => entry.toolName === 'desktop_capture_action');
  assert(actionResult && !actionResult.isError, JSON.stringify(actionResult));
  assert.equal(JSON.parse(actionResult.content.find(block => block.type === 'text').text).effect, 'handled');
  assert(results.some(entry => JSON.stringify(entry).includes('Desktop recovery required')));
  assert(results.some(entry => JSON.stringify(entry).includes('"type":"image"')));
  assert([...records.values()].some(record => record.kind === 'echo' && record.text.includes('"type":"image"')));
  const state = await runtime.harness.snapshot(runtime.DesktopState, runtime.id, runtime.context);
  assert.equal(state.recoveryRequired, true);
  assert.equal(state.captureId, null);
  await writeFile(join(evidence, 'pi-result.json'), JSON.stringify({ passed: true, calls, results: results.length, imagesRetained: true, repeatedActionBlocked: true, recoveryRequired: true }, null, 2));
} finally { await runtime.close(); }

// Exercise the completed-text adapter against the actual Rust decoder and action authority.
const textFaux = fauxProvider({ models: [{ id: 'text-live-fixture', contextWindow: 200000 }] });
const textModels = createModels(); textModels.setProvider(textFaux.provider);
textFaux.setResponses([
  toolRound([['desktop_observe', { output, window_id: Number(window), scale: 0.5 }]]),
  fauxAssistantMessage("Action: click(start_box='(500,500)')"),
  fauxAssistantMessage("Action: finished(content='Text profile acceptance finished')"),
]);
const textCalls = [];
const textRuntime = await PiRuntime.open({ stateDir: join(evidence, 'pi-text-profile'), root, memory: {
  async append() {}, async readyView() { return '<chat>\n</chat>'; }, async zoom() { return 'fixture'; }, async date() { return '2026-10-12'; },
}, config: { models: textModels, provider: 'faux', model: 'text-live-fixture', desktopModelProfile: 'ui_tars',
  instructions: 'Isolated textual computer profile fixture.', desktopClientFactory: async () => {
    const client = await DesktopMcp.open(root);
    const call = client.call.bind(client);
    client.call = async (method, params, options) => {
      if (method === 'tools/call') textCalls.push(params.name);
      return call(method, params, options);
    };
    return client;
  },
} });
try {
  await textRuntime.submit('Observe and execute one scripted textual prediction', { requestId: 'text-capture-live' });
  await textRuntime.queue; await textRuntime.conversation.waitForIdle(textRuntime.context); await textRuntime.archived;
  assert.deepEqual(textCalls, ['desktop_driver_tools', 'desktop_observe', 'desktop_decode_model', 'desktop_capture_action', 'desktop_decode_model', 'desktop_action']);
  const entries = await textRuntime.conversation.entries({ order: 'ascending' }, 100, undefined, textRuntime.context);
  const call = entries.items.flatMap(entry => entry.kind === 'pi.assistant' ? entry.model?.[0]?.content ?? [] : [])
    .find(block => block.type === 'toolCall' && block.name === 'desktop_capture_action');
  assert.equal(call.arguments.model_source.profile, 'ui_tars');
  assert.equal(call.arguments.model_source.raw, "Action: click(start_box='(500,500)')");
  assert.match(call.arguments.model_source.capture_digest, /^[0-9a-f]{64}$/);
  const result = entries.items.flatMap(entry => entry.kind === 'pi.tool-result' ? entry.model ?? [] : [])
    .find(message => message.toolName === 'desktop_capture_action');
  assert(result && !result.isError, JSON.stringify(result));
  const receipt = JSON.parse(result.content.find(block => block.type === 'text').text);
  assert.equal(receipt.effect, 'handled');
  assert(receipt.post_observation?.capture_id);
  assert(result.content.some(block => block.type === 'image'));
  await writeFile(join(evidence, 'pi-model-result.json'), JSON.stringify({ passed: true, calls: textCalls,
    sourceRetainedBeforeEffect: true, rustDecode: true, handled: true, freshImage: true }, null, 2));
} finally { await textRuntime.close(); }
