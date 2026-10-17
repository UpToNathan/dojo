import { mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { DojoCredentials } from './credentials.mjs';
export { DojoCredentials } from './credentials.mjs';
import { createModels } from '@earendil-works/pi-ai/models';
import { openaiCodexProvider } from '@earendil-works/pi-ai/providers/openai-codex';
import { openaiProvider } from '@earendil-works/pi-ai/providers/openai';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';
import { opencodeProvider } from '@earendil-works/pi-ai/providers/opencode';
import { opencodeGoProvider } from '@earendil-works/pi-ai/providers/opencode-go';

export const SUBSCRIPTION_PROVIDERS = ['openai-codex', 'openai', 'anthropic', 'opencode', 'opencode-go'];
// Ambient environment variables never authenticate a provider; only stored or explicitly imported credentials do.
const NO_AMBIENT_AUTH = { env: async () => undefined, fileExists: async () => false };

function cleanMarks(value) {
  if (Array.isArray(value)) return value.map(cleanMarks);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value).filter(([key]) => key !== 'cache_control' && key !== 'prompt_cache_breakpoint').map(([key, item]) => [key, cleanMarks(item)]));
}

export function viewBlocks(text) {
  const match = /^<chat>\n([\s\S]*?)\n<\/chat>([\s\S]*)$/.exec(text);
  if (!match) return null;
  const lines = match[1] ? match[1].split('\n') : [];
  const blocks = [];
  for (let index = 0; index < lines.length; index += 4) {
    blocks.push({ text: `${index === 0 ? '<chat>\n' : ''}${lines.slice(index, index + 4).join('\n')}\n`, whole: index + 4 <= lines.length });
  }
  if (!blocks.length) blocks.push({ text: '<chat>\n', whole: false });
  blocks.push({ text: `</chat>${match[2]}`, whole: false });
  return blocks;
}

/**
 * Chooses cache handling from the request's API, never from the provider id: OpenCode serves several protocols.
 * Shapes Dojo has no cache strategy for pass through unmarked.
 */
export function cacheShape(kind) {
  if (kind === 'anthropic' || kind === 'anthropic-messages') return 'anthropic';
  if (['responses', 'openai-codex', 'openai', 'openai-responses', 'openai-codex-responses'].includes(kind)) return 'responses';
  return 'passthrough';
}

export function cachePayload(kind, original, { cacheId, openaiExplicit = false, claudeSubscription = true } = {}) {
  const payload = cleanMarks(original);
  const shape = cacheShape(kind);
  if (shape === 'passthrough') return payload;
  const provider = shape === 'anthropic' ? 'anthropic' : 'responses';
  const anthropic = shape === 'anthropic';
  if (!anthropic) {
    payload.prompt_cache_key = cacheId;
    if (openaiExplicit) payload.prompt_cache_options = { mode: 'implicit' };
  }
  const messages = anthropic ? payload.messages : payload.input;
  const expanded = [];
  for (const message of messages || []) {
    expanded.push(message);
    if (message.role !== 'user') continue;
    const content = typeof message.content === 'string' ? message.content : (message.content || []).map(block => block.text || '').join('');
    const blocks = viewBlocks(content);
    if (!blocks || Array.isArray(message.content) && message.content.some(block => !['text', 'input_text'].includes(block.type))) continue;
    const lastWhole = blocks.findLastIndex(block => block.whole);
    message.content = blocks.map((block, index) => ({ type: anthropic ? 'text' : 'input_text', text: block.text,
      ...(index === lastWhole ? anthropic ? { cache_control: { type: 'ephemeral', ttl: claudeSubscription ? '1h' : '5m' } }
        : openaiExplicit ? { prompt_cache_breakpoint: { mode: 'explicit' } } : {} : {}) }));
    if (!anthropic && !openaiExplicit) {
      expanded.pop();
      expanded.push(...message.content.map(block => ({ ...message, content: [block] })));
    }
  }
  if (!anthropic) payload.input = expanded;
  if (anthropic) {
    const mark = { type: 'ephemeral', ttl: claudeSubscription ? '1h' : '5m' };
    if (payload.tools?.length) payload.tools.at(-1).cache_control = mark;
    if (Array.isArray(payload.system) && payload.system.length) payload.system.at(-1).cache_control = mark;
    const last = payload.messages?.findLast(message => message.role === 'user');
    if (last) {
      if (typeof last.content === 'string') last.content = [{ type: 'text', text: last.content }];
      const block = last.content?.at(-1);
      if (block && ['text', 'tool_result'].includes(block.type)) block.cache_control = mark;
    }
  }
  return payload;
}

function prefixKey(provider, model, payload) {
  if (cacheShape(provider) === 'passthrough') return null;
  const messages = payload.messages || payload.input || [];
  const first = messages.find(message => message.role === 'user');
  const blocks = first?.content;
  const boundary = Array.isArray(blocks) ? blocks.findLastIndex(block => block.cache_control || block.prompt_cache_breakpoint) : -1;
  if (boundary < 0 && provider !== 'anthropic') {
    const whole = [];
    let started = false;
    for (const message of messages) {
      const text = message.content?.map?.(block => block.text || '').join('') || '';
      if (!started && text.startsWith('<chat>\n')) started = true;
      if (!started) continue;
      if (text.includes('</chat>')) break;
      const lines = text.replace(/^<chat>\n/, '').split('\n').filter(Boolean);
      if (lines.length === 4) whole.push(message);
    }
    if (!whole.length) return null;
    return createHash('sha256').update(JSON.stringify([provider, model.id, payload.prompt_cache_key,
      payload.instructions, payload.tools, whole])).digest('hex');
  }
  if (boundary < 0) return null;
  return createHash('sha256').update(JSON.stringify([provider, model.id, payload.prompt_cache_key,
    payload.system || payload.instructions, payload.tools, blocks.slice(0, boundary + 1)])).digest('hex');
}

export function wrapCacheProvider(base, options) {
  const writes = new Map();
  const wrap = method => (model, context, requestOptions = {}) => {
    let release, key, gate;
    const unlock = () => {
      if (!release) return;
      release(); release = undefined;
      if (writes.get(key) === gate) writes.delete(key);
    };
    const shape = cacheShape(model.api ?? base.id);
    const firstParty = base.id === 'anthropic';
    const stream = base[method](model, context, { ...requestOptions, cacheRetention: firstParty ? 'long' : 'short',
      sessionId: options.cacheId,
      onPayload: async original => {
        const previous = await requestOptions.onPayload?.(original, model);
        const payload = cachePayload(shape, previous ?? original, { ...options, claudeSubscription: firstParty && options.claudeSubscription !== false });
        key = prefixKey(shape, model, payload);
        if (key) {
          const previous = writes.get(key);
          gate = new Promise(resolve => { release = resolve; });
          writes.set(key, gate);
          if (previous) {
            const signal = requestOptions.signal;
            signal?.throwIfAborted();
            let abort;
            const canceled = new Promise((_, reject) => {
              abort = () => reject(signal.reason || new Error('Cache-prefix wait canceled.'));
              signal?.addEventListener('abort', abort, { once: true });
            });
            try { await Promise.race([previous, canceled]); }
            finally { signal?.removeEventListener('abort', abort); }
          }
        }
        await options.onPayload?.(payload, model);
        return payload;
      },
      onProviderStreamEvent: async (event, actualModel) => {
        unlock();
        await requestOptions.onProviderStreamEvent?.(event, actualModel);
      },
    });
    stream.result().finally(unlock).catch(() => {});
    return stream;
  };
  return { ...base, stream: wrap('stream'), streamSimple: wrap('streamSimple') };
}

export function subscriptionProvider(provider) {
  if (!SUBSCRIPTION_PROVIDERS.includes(provider)) throw new Error('Unsupported Dojo account.');
  return { anthropic: anthropicProvider, openai: openaiProvider, 'openai-codex': openaiCodexProvider,
    opencode: opencodeProvider, 'opencode-go': opencodeGoProvider }[provider]();
}

/**
 * Registers one provider per usable account in a single catalog. `providers` lists accounts in preference order;
 * accounts without a credential are skipped, and it is an error only when none remain.
 */
export async function createDojoModels({ stateDir, provider = 'openai-codex', providers, cacheId = 'dojo', onPayload,
  openaiExplicit = false, credentials: supplied, store, importCredentials = true, env } = {}) {
  const wanted = providers ?? [provider];
  for (const id of wanted) if (!SUBSCRIPTION_PROVIDERS.includes(id)) throw new Error('Unsupported Dojo account.');
  await mkdir(stateDir, { recursive: true, mode: 0o700 });
  // `credentials` is a caller-managed store that is used as given; `store` is a Dojo store that is still seeded.
  const credentials = supplied || store || new DojoCredentials(stateDir);
  const usable = [];
  for (const id of wanted) {
    if (importCredentials && !supplied) {
      try { await credentials.importAccount(id, { env }); }
      catch (error) { if (providers === undefined) throw error; continue; }
    }
    let credential;
    try { credential = await credentials.read(id); } catch (error) { if (providers === undefined) throw error; continue; }
    if (!supplied && !['oauth', 'api_key'].includes(credential?.type)) {
      if (providers === undefined) throw new Error('Dojo requires subscription authentication.');
      continue;
    }
    usable.push(id);
  }
  if (!supplied && wanted.length > 0 && !usable.length) throw new Error('Sign in to use Dojo: no account is available.');
  const models = createModels({ credentials, authContext: NO_AMBIENT_AUTH });
  for (const id of supplied ? wanted : usable) {
    models.setProvider(wrapCacheProvider(subscriptionProvider(id), { cacheId, onPayload, openaiExplicit,
      claudeSubscription: id === 'anthropic' }));
  }
  models.dojoAccounts = supplied ? wanted : usable;
  return models;
}
