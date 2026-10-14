import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { createHash } from 'node:crypto';

// pi-ai Usage.input excludes cacheRead/cacheWrite (models.js calculateCost;
// anthropic-messages and openai-responses-shared provider normalization).
export function desktopTraceUsage(entries, conversationId) {
  if (conversationId === undefined || conversationId === null || !String(conversationId).length || String(conversationId).length > 256) throw new Error('Usage requires a conversation identity');
  const samples = [];
  for (const entry of entries) {
    if (entry.kind !== 'pi.assistant') continue;
    const usage = entry.model?.[0]?.usage;
    if (usage === undefined || usage === null) continue;
    if (entry.id === undefined || entry.id === null || String(entry.id).length > 1024) throw new Error('Usage requires a canonical entry identity');
    const counters = [usage.input, usage.output, usage.cacheRead, usage.cacheWrite];
    if (!counters.every(value => Number.isSafeInteger(value) && value >= 0)) throw new Error('Invalid canonical token usage');
    const input = usage.input + usage.cacheRead + usage.cacheWrite;
    if (!Number.isSafeInteger(input) || !Number.isSafeInteger(input + usage.output)) throw new Error('Token usage exceeds integer precision');
    const cost = usage.cost?.total;
    const micros = typeof cost === 'number' && Number.isFinite(cost) && cost >= 0 ? Math.round(cost * 1_000_000) : NaN;
    samples.push({
      receipt_digest: createHash('sha256').update(JSON.stringify(['dojo.pi.usage/1', String(conversationId), String(entry.id)])).digest('hex'),
      input_tokens: input, output_tokens: usage.output,
      cache_read_tokens: usage.cacheRead, cache_write_tokens: usage.cacheWrite,
      cost_microusd: Number.isSafeInteger(micros) ? micros : null,
    });
    if (samples.length > 4096) throw new Error('Desktop usage sample budget exceeded');
  }
  return samples;
}

// Only Pi-owned entries are projected. The Rust codec redacts before export.
const desktopRoute = name => typeof name === 'string' && (name.startsWith('desktop_') || name.startsWith('computer_steps_'));

export function desktopTraceEvents(entries) {
  const events = [];
  const calls = new Map();
  let epoch, elapsed = 0;
  for (const entry of entries) {
    const message = entry.model?.[0];
    if (Number.isSafeInteger(message?.timestamp) && message.timestamp >= 0) {
      epoch ??= message.timestamp;
      elapsed = Math.max(elapsed, message.timestamp - epoch);
    }
    if (entry.kind === 'pi.assistant') {
      for (const call of message?.content ?? []) {
        if (call.type !== 'toolCall' || !desktopRoute(call.name)) continue;
        calls.set(call.id, call.name);
        events.push({ time_ms: elapsed, kind: 'intent', pi_intent_id: String(entry.id), call_id: call.id,
          route: call.name, observation_id: call.arguments?.capture_id ?? call.arguments?.action?.capture_id ?? null, effect: null, data: { arguments: call.arguments } });
      }
    } else if (entry.kind === 'pi.tool-result') {
      const route = message?.toolName ?? calls.get(message?.toolCallId);
      if (!desktopRoute(route)) continue;
      let data = {};
      for (const block of message?.content ?? []) if (block.type === 'text') {
        try { data = JSON.parse(block.text); break; } catch {}
      }
      const effect = message.isError ? 'refused' : data.effect === 'partial' ? 'partial' : data.effect === 'refused' ? 'refused' : 'unknown';
      // Delivery acknowledgements cannot establish verified UI success.
      events.push({ time_ms: elapsed, kind: 'receipt', pi_intent_id: String(entry.id), call_id: message.toolCallId ?? null,
        route, observation_id: data.capture_id ?? null, effect, data });
    }
    if (events.length > 1000) throw new Error('Desktop export exceeds 1000 events; select a smaller conversation');
  }
  return events;
}

export async function exportDesktopTrace(runtime, { secretValues = [], timeoutMs = 5000 } = {}) {
  if (!Number.isFinite(timeoutMs) || timeoutMs < 1 || timeoutMs > 30000) throw new Error('Invalid export deadline');
  const entries = [];
  let cursor;
  do {
    const page = await runtime.conversation.entries({ order: 'ascending' }, 200, cursor, runtime.context);
    entries.push(...page.items);
    if (entries.length > 10000) throw new Error('Desktop export entry budget exceeded');
    cursor = page.next;
  } while (cursor !== undefined);
  const usage = desktopTraceUsage(entries, runtime.id);
  const input = JSON.stringify({ command: 'record', session_id: String(runtime.id),
    policy: { max_events: 1000, max_event_bytes: 65536, max_total_bytes: 8388608, retain_images: false,
      max_images: 0, max_image_bytes: 1048576, max_image_pixels: 16777216, secret_values: secretValues },
    events: desktopTraceEvents(entries), ...(usage.length ? { usage } : {}) });
  if (Buffer.byteLength(input) > 8388608) throw new Error('Desktop export byte budget exceeded');
  return new Promise((resolve, reject) => {
    const child = spawn(join(runtime.root, 'target/debug/dojo-computer-recording'), [], { stdio: ['pipe', 'pipe', 'pipe'] });
    const output = []; let size = 0;
    const timer = setTimeout(() => { child.kill('SIGKILL'); reject(new Error('Desktop export timed out')); }, timeoutMs);
    child.on('error', error => { clearTimeout(timer); reject(error); });
    child.stdin.on('error', error => { child.kill('SIGKILL'); clearTimeout(timer); reject(error); });
    child.stderr.resume();
    child.stdout.on('data', chunk => {
      size += chunk.length;
      if (size > 8388608) { child.kill('SIGKILL'); reject(new Error('Desktop export output budget exceeded')); }
      else output.push(chunk);
    });
    child.on('close', code => {
      clearTimeout(timer);
      if (code !== 0) reject(new Error('Rust desktop trace codec refused export'));
      else { try { resolve(JSON.parse(Buffer.concat(output).toString())); } catch { reject(new Error('Invalid Rust desktop export')); } }
    });
    child.stdin.end(input);
  });
}
