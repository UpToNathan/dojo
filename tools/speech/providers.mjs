import { readFile, mkdir, writeFile, rename, chmod } from 'node:fs/promises';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { createHash, randomUUID } from 'node:crypto';
import { createModels } from '@earendil-works/pi-ai/models';
import { openaiCodexProvider } from '@earendil-works/pi-ai/providers/openai-codex';
import { openaiProvider } from '@earendil-works/pi-ai/providers/openai';
import { anthropicProvider } from '@earendil-works/pi-ai/providers/anthropic';

async function readJson(file, fallback) {
  try { return JSON.parse(await readFile(file, 'utf8')); }
  catch (error) { if (error.code === 'ENOENT') return fallback; throw error; }
}

export class DojoCredentials {
  constructor(stateDir) { this.file = join(stateDir, 'credentials.json'); this.chain = Promise.resolve(); }
  async read(provider) {
    await this.chain;
    const value = (await readJson(this.file, {}))[provider];
    if (value?.borrowedSubscription) {
      const fresh = await this.subscriptionSnapshot(provider);
      if (fresh.expires <= Date.now() + 60_000) throw new Error('The shared subscription login needs renewal in its owning app, or a separate Dojo login. Dojo will not rotate another app\'s credentials.');
      return fresh;
    }
    return value;
  }
  async list() { await this.chain; return Object.entries(await readJson(this.file, {})).map(([providerId, value]) => ({ providerId, type: value.type })); }
  modify(provider, change, { signal } = {}) {
    const work = this.chain.then(async () => {
      signal?.throwIfAborted();
      const all = await readJson(this.file, {});
      const value = await change(all[provider]);
      signal?.throwIfAborted();
      if (value === undefined) return all[provider];
      all[provider] = value;
      const temporary = `${this.file}.${randomUUID()}.tmp`;
      await writeFile(temporary, JSON.stringify(all), { mode: 0o600, flag: 'wx' });
      await rename(temporary, this.file);
      await chmod(this.file, 0o600);
      return value;
    });
    this.chain = work.catch(() => {});
    return work;
  }
  delete(provider) {
    const work = this.chain.then(async () => {
      const all = await readJson(this.file, {}); delete all[provider];
      await writeFile(this.file, JSON.stringify(all), { mode: 0o600 });
    });
    this.chain = work.catch(() => {}); return work;
  }
  async importSubscription(provider) {
    if (await this.read(provider)) return;
    await this.modify(provider, async () => await this.subscriptionSnapshot(provider));
  }
  async subscriptionSnapshot(provider) {
    if (provider === 'openai-codex') {
      const auth = await readJson(join(homedir(), '.codex/auth.json'), null);
      const tokens = auth?.tokens;
      if (!tokens?.access_token) throw new Error('Sign in to Codex before enabling Dojo subscription execution.');
      const claims = JSON.parse(Buffer.from(tokens.access_token.split('.')[1], 'base64url'));
      if (!Number.isFinite(claims.exp)) throw new Error('Codex subscription credential has no expiry.');
      return { type: 'oauth', access: tokens.access_token, refresh: '', borrowedSubscription: true,
        expires: claims.exp * 1000, accountId: tokens.account_id };
    } else {
      const auth = await readJson(join(homedir(), '.pi/agent/auth.json'), {});
      if (auth[provider]?.type !== 'oauth') throw new Error(`Dojo needs a ${provider} subscription login; API-key fallback is disabled.`);
      if (typeof auth[provider].access !== 'string' || !auth[provider].access || !Number.isFinite(auth[provider].expires)) throw new Error('Invalid subscription access credential.');
      return { ...auth[provider], refresh: '', borrowedSubscription: true };
    }
  }
}

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

export function cachePayload(provider, original, { cacheId, openaiExplicit = false, claudeSubscription = true } = {}) {
  const payload = cleanMarks(original);
  const anthropic = provider === 'anthropic';
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
    const stream = base[method](model, context, { ...requestOptions, cacheRetention: base.id === 'anthropic' ? 'long' : 'short',
      sessionId: options.cacheId,
      onPayload: async original => {
        const previous = await requestOptions.onPayload?.(original, model);
        const payload = cachePayload(base.id, previous ?? original, options);
        key = prefixKey(base.id, model, payload);
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
  if (!['openai-codex', 'openai', 'anthropic'].includes(provider)) throw new Error('Unsupported Dojo subscription provider.');
  return provider === 'anthropic' ? anthropicProvider() : provider === 'openai' ? openaiProvider() : openaiCodexProvider();
}

export async function createDojoModels({ stateDir, provider = 'openai-codex', cacheId = 'dojo', onPayload,
  openaiExplicit = false, credentials: supplied, importCredentials = true } = {}) {
  if (!['openai-codex', 'openai', 'anthropic'].includes(provider)) throw new Error('Unsupported Dojo subscription provider.');
  await mkdir(stateDir, { recursive: true, mode: 0o700 });
  const credentials = supplied || new DojoCredentials(stateDir);
  if (importCredentials && !supplied) await credentials.importSubscription(provider);
  const credential = await credentials.read(provider);
  if (!supplied && credential?.type !== 'oauth') throw new Error('Dojo requires subscription authentication.');
  const base = subscriptionProvider(provider);
  const models = createModels({ credentials });
  models.setProvider(wrapCacheProvider(base, { cacheId, onPayload, openaiExplicit, claudeSubscription: true }));
  return models;
}
