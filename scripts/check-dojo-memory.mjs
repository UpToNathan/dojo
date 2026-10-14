#!/usr/bin/env node
import assert from 'node:assert/strict';
import { mkdir, readFile, writeFile, mkdtemp } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { tmpdir, homedir } from 'node:os';
import { randomUUID, createHash } from 'node:crypto';
import { MemoryStore } from '../tools/speech/memory.mjs';
import { PiRuntime } from '../tools/speech/pi-runtime.mjs';

const root = resolve(new URL('..', import.meta.url).pathname);
const evidence = process.argv[2] || await mkdtemp(join(tmpdir(), 'dojo-pi-live-'));
await mkdir(evidence, { recursive: true, mode: 0o700 });
const protectedFiles = [join(homedir(), '.codex/auth.json'), join(homedir(), '.codex/config.toml'), join(homedir(), '.pi/agent/auth.json'), join(root, '.codex/config.toml'), join(root, 'AGENTS.md')];
async function hashes() {
  const result = {};
  for (const file of protectedFiles) { try { result[file] = createHash('sha256').update(await readFile(file)).digest('hex'); } catch (error) { if (error.code !== 'ENOENT') throw error; result[file] = null; } }
  return result;
}
const before = await hashes(), requests = [], responses = [];
let memory = await MemoryStore.open(join(evidence, 'memory'));
const config = { model: 'gpt-6-luna', summaryModel: 'gpt-6-luna', desktopTools: false,
  onPayload: (payload, model) => requests.push({ model: model.id, instructionsHash: createHash('sha256').update(payload.instructions || JSON.stringify(payload.system)).digest('hex'), toolsHash: createHash('sha256').update(JSON.stringify(payload.tools)).digest('hex'), cacheId: payload.prompt_cache_key, inputItems: payload.input?.length, input: payload.input }) };
let runtime = await PiRuntime.open({ stateDir: join(evidence, 'pi'), root, memory, config, onEvent: event => { if (event.type === 'response') responses.push(event.text); } });
const marker = `cedar-${randomUUID()}`;
async function finish() { await runtime.queue; await runtime.conversation.waitForIdle(runtime.context); await runtime.archived; }
try {
  await runtime.submit(`This is a temporary memory test, not a lasting preference. Remember the exact marker ${marker}. No tools or file changes. Reply only: stored. ${'This padding adds no instruction or fact. '.repeat(18)}`, { requestId: 'live-first' });
  await finish();
  assert.ok(['stored', 'stored.'].includes(responses.at(-1)));
  const original = await memory.zoom(0);
  assert.ok(original.text.includes(marker));
  await memory.readyView(runtime.summarize);
  const id = runtime.id, stableView = memory.view();
  await runtime.close(); await memory.close();
  memory = await MemoryStore.open(join(evidence, 'memory'));
  assert.equal(memory.view(), stableView);
  runtime = await PiRuntime.open({ stateDir: join(evidence, 'pi'), root, memory, config, onEvent: event => { if (event.type === 'response') responses.push(event.text); } });
  assert.equal(runtime.id, id);
  assert.equal((await memory.zoom(0)).text, original.text);
  responses.length = 0;
  await runtime.submit('What exact marker did I give you? First call zoom with id 0 and n 1 to inspect the original message. Reply only with that marker. No desktop actions or files.', { requestId: 'live-second' });
  await finish();
  assert.ok([marker, `${marker}.`].includes(responses.at(-1)));
  const page = await runtime.conversation.entries({ order: 'ascending' }, 200, undefined, runtime.context);
  assert.ok(page.items.some(entry => entry.kind === 'pi.assistant' && entry.model?.[0]?.content?.some(block => block.type === 'toolCall' && block.name === 'zoom')));
  assert.equal(new Set(requests.map(request => request.instructionsHash)).size, 1);
  assert.equal(new Set(requests.map(request => request.toolsHash)).size, 1);
  assert.equal(new Set(requests.map(request => request.cacheId)).size, 1);
  const after = await hashes(); assert.deepEqual(after, before);
  const result = { passed: true, marker, conversationId: id, restartPreservedSource: true, restartPreservedView: true, actualZoom: true, stableInstructions: true, stableTools: true, stableCacheIdentity: true, protectedFilesUnchanged: true, requestCount: requests.length };
  await writeFile(join(evidence, 'result.json'), JSON.stringify(result, null, 2), { mode: 0o600 });
  await writeFile(join(evidence, 'requests.json'), JSON.stringify(requests, null, 2), { mode: 0o600 });
  console.log(JSON.stringify(result));
} finally { await runtime.close(); await memory.close(); }
