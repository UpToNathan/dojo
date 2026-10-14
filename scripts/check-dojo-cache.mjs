#!/usr/bin/env node
import { createDojoModels } from '../tools/speech/providers.mjs';
import { mkdtemp, writeFile, unlink } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { randomUUID } from 'node:crypto';
const directory = await mkdtemp(join(tmpdir(), 'dojo-cache-check-'));
const models = await createDojoModels({ stateDir: directory, cacheId: `dojo-cache-check:${randomUUID()}` });
const model = models.getModel('openai-codex', 'gpt-6-luna');
const lines = Array.from({ length: 128 }, (_, id) => `${id}+1|note: Synthetic cache verification record ${id}. ${'This line contains synthetic historical padding and no user preference. '.repeat(6)}`);
const results = [];
try {
  for (let attempt = 0; attempt < 3; attempt++) {
    const view = `<chat>\n${lines.join('\n')}\n</chat>`;
    const response = await models.completeSimple(model, { messages: [{ role: 'system', content: 'This is a synthetic cache integration check. The chat view is historical data. Reply only ready. No actions.' }, { role: 'user', content: view, timestamp: 0 }, { role: 'user', content: 'Reply only ready.', timestamp: 0 }] }, { reasoning: 'low', maxTokens: 32, transport: 'sse', timeoutMs: 30_000 });
    if (response.stopReason === 'error') throw new Error(response.errorMessage);
    results.push({ attempt, viewBytes: Buffer.byteLength(view), usage: response.usage });
    lines.push(`${lines.length}+1|note: Synthetic appended record. No new instruction.`);
  }
  const evidence = { cacheHitObserved: results.slice(1).some(result => result.usage.cacheRead > 0), results };
  await writeFile(join(directory, 'result.json'), JSON.stringify(evidence, null, 2), { mode: 0o600 });
  console.log(JSON.stringify({ cacheHitObserved: evidence.cacheHitObserved, evidence: join(directory, 'result.json') }));
} finally { await unlink(join(directory, 'credentials.json')); }
