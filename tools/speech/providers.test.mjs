import test from 'node:test';
import assert from 'node:assert/strict';
import { cachePayload, viewBlocks, wrapCacheProvider, DojoCredentials } from './providers.mjs';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';
import { mkdtemp, rm, stat, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const view = n => `<chat>\n${Array.from({ length: n }, (_, i) => `${i}+1|note: Fact ${i}`).join('\n')}\n</chat>`;
test('three matching writers hand off at response start without a gap', async () => {
  const launched = [], release = [];
  const base = { id: 'openai-codex', streamSimple(model, context, options) {
    const result = (async () => {
      await options.onPayload({ instructions: 'fixed', tools: [], input: [{ role: 'user', content: [{ type: 'input_text', text: view(4) }] }] });
      launched.push(context.id);
      await new Promise(resolve => { release[context.id] = resolve; });
      await options.onProviderStreamEvent({ type: 'response.created' }, model);
      return {};
    })();
    return { result: () => result };
  } };
  const provider = wrapCacheProvider(base, { cacheId: 'same' });
  const streams = [0, 1, 2].map(id => provider.streamSimple({ id: 'model' }, { id }));
  await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(launched, [0]);
  release[0](); await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(launched, [0, 1]);
  release[1](); await new Promise(resolve => setImmediate(resolve)); assert.deepEqual(launched, [0, 1, 2]);
  release[2](); await Promise.all(streams.map(stream => stream.result()));
});
test('four-line blocks preserve exact view bytes and growing whole prefixes', () => {
  assert.equal(viewBlocks(view(9)).map(block => block.text).join(''), view(9));
  assert.deepEqual(viewBlocks(view(9)).filter(block => block.whole), viewBlocks(view(8)).filter(block => block.whole));
});
test('OpenAI subscription preserves cache identity and reusable message boundaries without unsupported flags', () => {
  const input = { instructions: 'fixed', tools: [{ name: 'zoom' }], input: [{ role: 'user', content: [{ type: 'input_text', text: view(8) }] }, { role: 'user', content: [{ type: 'input_text', text: 'current request' }] }] };
  const first = cachePayload('openai-codex', input, { cacheId: 'dojo-account' });
  const next = cachePayload('openai-codex', { ...input, input: [{ role: 'user', content: [{ type: 'input_text', text: view(9) }] }, input.input[1]] }, { cacheId: 'dojo-account' });
  assert.equal(first.prompt_cache_key, 'dojo-account');
  assert.equal(first.prompt_cache_options, undefined);
  assert.deepEqual(first.input.slice(0, 2), next.input.slice(0, 2));
  assert.deepEqual(first.tools, input.tools);
  assert.equal(first.instructions, 'fixed');
  assert.equal(first.input.at(-1).content[0].text, 'current request');
});
test('Claude marks only the fixed envelope, last complete view block and request end, preserving OAuth identity', () => {
  const payload = cachePayload('anthropic', { system: [{ type: 'text', text: 'Claude Code identity' }, { type: 'text', text: 'fixed prompt' }], tools: [{ name: 'zoom' }, { name: 'date' }], messages: [{ role: 'user', content: view(9) }, { role: 'user', content: 'request' }] });
  assert.equal(payload.system[0].text, 'Claude Code identity');
  assert.equal(payload.system[0].cache_control, undefined);
  const marked = payload.messages[0].content.filter(block => block.cache_control);
  assert.equal(marked.length, 1);
  assert.match(marked[0].text, /^4\+1\|/);
  assert.equal(payload.tools.at(-1).cache_control.ttl, '1h');
  assert.equal(payload.messages.at(-1).content.at(-1).cache_control.ttl, '1h');
  const api = cachePayload('anthropic', { messages: [{ role: 'user', content: view(4) }] }, { claudeSubscription: false });
  assert.equal(api.messages[0].content.at(-1).cache_control.ttl, '5m');
});
test('real Anthropic OAuth adapter passes the transformed four-line blocks to its send boundary', async () => {
  let captured;
  const base = anthropicProvider();
  const model = base.getModels().find(model => model.id === 'claude-haiku-5-5');
  assert.ok(model);
  const provider = wrapCacheProvider(base, { cacheId: 'dojo-test', onPayload: payload => { captured = payload; throw new Error('Captured before network'); } });
  const answer = await provider.streamSimple(model, { messages: [{ role: 'system', content: 'fixed prompt' }, { role: 'user', content: view(8), timestamp: 0 }] }, { apiKey: 'sk-ant-oat01-synthetic-test-token', reasoning: 'xhigh' }).result();
  assert.match(answer.errorMessage, /Captured before network/);
  assert.ok(captured.system.some(block => block.text.includes('Claude Code')));
  assert.equal(captured.messages[0].content.filter(block => block.cache_control).length, 2);
});
test('Dojo credential updates are private and serialized without rewriting another store', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'dojo-auth-test-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const credentials = new DojoCredentials(directory);
  await Promise.all([credentials.modify('a', async () => ({ type: 'oauth', access: 'a' })), credentials.modify('b', async () => ({ type: 'oauth', access: 'b' }))]);
  assert.equal((await credentials.read('a')).access, 'a');
  assert.equal((await credentials.read('b')).access, 'b');
  assert.equal((await stat(join(directory, 'credentials.json'))).mode & 0o777, 0o600);
  assert.deepEqual(Object.keys(JSON.parse(await readFile(join(directory, 'credentials.json'), 'utf8'))), ['a', 'b']);
});
