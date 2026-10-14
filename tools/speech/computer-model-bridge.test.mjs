import test from 'node:test';
import assert from 'node:assert/strict';
import { computerModelBridge, decodedAction } from './computer-model-bridge.mjs';
import { fauxAssistantMessage } from '@earendil-works/pi-ai/providers/faux';

function foregroundBridge(models, runtime) {
  const view = { role: 'user', content: '<chat>\n</chat>', timestamp: 0 };
  return { bridge: computerModelBridge(models, runtime, 'ui_tars', new WeakSet([view])), context: { messages: [view] } };
}

const at = { delivered_x: 0, delivered_y: 3.5 };
const decoded = outcome => ({ capture_id: 'owned', image_width: 100, image_height: 80, outcome });

test('same-ID unmarked summary keeps provider stream and never accesses desktop state', async () => {
  const summary = fauxAssistantMessage('plain summary');
  const stream = { result: async () => summary };
  const models = { streamSimple: () => stream };
  const runtime = { agent: { model: { modelId: 'test' } }, harness: { snapshot: async () => { throw new Error('Summary accessed desktop'); } } };
  const marked = { role: 'user', content: '<chat>\n</chat>', timestamp: 0 };
  const bridge = computerModelBridge(models, runtime, 'ui_tars', new WeakSet([marked]));
  const actual = bridge.streamSimple({ id: 'test' }, { messages: [{ ...marked }] });
  assert.equal(actual, stream);
  assert.equal(await actual.result(), summary);
});

test('mapping preserves zero and fractional pixels and refuses lossy actions', () => {
  assert.deepEqual(decodedAction(decoded({ Action: { Click: { at, button: 'Left', count: 1 } } })), {
    capture_id: 'owned', observe_after: true, x: 0, y: 3.5, input: { kind: 'click', button: 'left' },
  });
  assert.throws(() => decodedAction(decoded({ Action: { Click: { at, button: 'Left', count: 3 } } })), /click count/);
  assert.throws(() => decodedAction(decoded({ Action: { Scroll: { at, dx: 1, dy: 2, unit: 'WheelSteps' } } })), /Wheel-step/);
});

test('text prediction becomes a Pi tool call before the completed response is classified', async () => {
  const original = fauxAssistantMessage("click(x=0,y=3.5)");
  const models = { streamSimple: () => ({ result: async () => original }) };
  let requests = 0;
  const runtime = { id: 'conversation', context: {}, agent: { model: { modelId: 'test' } }, DesktopState: {}, harness: { snapshot: async () => ({ captureId: 'owned', recoveryRequired: false }) }, desktop: {}, decodeDesktopModel: async params => {
    requests++; assert.equal(params.capture_id, 'owned');
    return { content: [{ type: 'text', text: JSON.stringify(decoded({ Action: { Click: { at, button: 'Left', count: 1 } } })) }] };
  } };
  const { bridge, context } = foregroundBridge(models, runtime);
  const message = await bridge.streamSimple({ id: 'test' }, context).result();
  assert.equal(requests, 1);
  assert.equal(message.stopReason, 'toolUse');
  assert.equal(message.content[0].name, 'desktop_capture_action');
  assert.equal(message.content[0].arguments.capture_id, 'owned');
  assert.equal(original.content[0].type, 'text');
});

test('missing fresh observation refuses without sending Rust decode or effects', async () => {
  const models = { stream: () => ({ result: async () => fauxAssistantMessage('click()') }) };
  const runtime = { agent: { model: { modelId: 'test' } }, harness: { snapshot: async () => ({ recoveryRequired: true }) } };
  const { bridge, context } = foregroundBridge(models, runtime);
  const message = await bridge.stream({ id: 'test' }, context).result();
  assert.equal(message.stopReason, 'error');
  assert.match(message.errorMessage, /Fresh desktop observation/);
});

 test('model clarification becomes a durable human request tool call', async () => {
  const models = { stream: () => ({ result: async () => fauxAssistantMessage('call_user()') }) };
  const runtime = { id: 'c', agent: { model: { modelId: 'test' } }, desktop: {}, harness: { snapshot: async () => ({ captureId: 'owned' }) },
    decodeDesktopModel: async () => ({ content: [{ type: 'text', text: JSON.stringify(decoded({ AskHuman: { text: 'Which document?' } })) }] }) };
  const { bridge, context } = foregroundBridge(models, runtime);
  const message = await bridge.stream({ id: 'test' }, context).result();
  assert.equal(message.stopReason, 'toolUse');
  assert.equal(message.content[0].name, 'human_request');
  assert.deepEqual(message.content[0].arguments, { prompt: 'Which document?' });
});
