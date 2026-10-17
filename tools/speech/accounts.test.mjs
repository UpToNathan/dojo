import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { ACCOUNT_ORDER, discoverAccounts, listModels, defaultSelection, summaryModel, prepareAgentModels } from './accounts.mjs';
import { DojoCredentials, cachePayload, cacheShape, createDojoModels, wrapCacheProvider } from './providers.mjs';

const secret = 'sk-test-secret-value-0123456789';
const fileCredentials = state => new DojoCredentials(state, { command: async () => { throw Object.assign(new Error('org.freedesktop.DBus.Error.ServiceUnknown'), { code: 1 }); } });

async function isolatedHome(t, { piAuth, codexAuth } = {}) {
  const home = await mkdtemp(join(tmpdir(), 'dojo-accounts-home-'));
  const previous = process.env.HOME;
  process.env.HOME = home;
  t.after(async () => { process.env.HOME = previous; await rm(home, { recursive: true, force: true }); });
  if (piAuth) { await mkdir(join(home, '.pi/agent'), { recursive: true }); await writeFile(join(home, '.pi/agent/auth.json'), JSON.stringify(piAuth)); }
  if (codexAuth) { await mkdir(join(home, '.codex'), { recursive: true }); await writeFile(join(home, '.codex/auth.json'), JSON.stringify(codexAuth)); }
  const state = join(home, 'state');
  await mkdir(state, { recursive: true });
  return { home, state };
}
const unsignedJwt = expires => `x.${Buffer.from(JSON.stringify({ exp: expires })).toString('base64url')}.y`;

test('no accounts are available without a stored or explicitly imported credential', async t => {
  const { state } = await isolatedHome(t);
  const accounts = await discoverAccounts({ credentials: fileCredentials(state), env: {} });
  assert.deepEqual(accounts.map(account => account.id), ACCOUNT_ORDER);
  assert.ok(accounts.every(account => account.state === 'unavailable' && account.reason));
});

test('an ambient OPENCODE_API_KEY never makes an account available', async t => {
  const { state } = await isolatedHome(t);
  const accounts = await discoverAccounts({ credentials: fileCredentials(state), env: { OPENCODE_API_KEY: secret } });
  assert.ok(accounts.every(account => account.state === 'unavailable'));
  const prepared = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: { OPENCODE_API_KEY: secret } });
  assert.equal(prepared.selection, null);
  assert.equal(prepared.rows.length, 0);
});

test('an OpenCode key in the Pi login or DOJO_OPENCODE_API_KEY is adopted without leaking', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const imported = await discoverAccounts({ credentials: fileCredentials(state), env: {} });
  const go = imported.find(account => account.id === 'opencode-go');
  assert.equal(go.state, 'available');
  assert.equal(go.source, 'import');
  assert.equal(imported.find(account => account.id === 'opencode').state, 'unavailable');
  assert.ok(!JSON.stringify(imported).includes(secret));

  const explicit = await discoverAccounts({ credentials: fileCredentials(state), env: { DOJO_OPENCODE_API_KEY: ' explicit-key ' } });
  assert.equal(explicit.find(account => account.id === 'opencode').state, 'available');
  assert.ok(!JSON.stringify(explicit).includes('explicit-key'));
});

test('a live borrowed ChatGPT login remains an import and is not persisted by discovery', async t => {
  const { state } = await isolatedHome(t, { codexAuth: { tokens: { access_token: unsignedJwt(Math.floor(Date.now() / 1000) + 3600), account_id: 'acct' } } });
  const account = (await discoverAccounts({ credentials: fileCredentials(state), env: {} })).find(account => account.id === 'openai-codex');
  assert.equal(account.state, 'available');
  assert.equal(account.source, 'borrowed');
  assert.equal(account.backend, undefined);
  await assert.rejects(readFile(join(state, 'credentials.json')), { code: 'ENOENT' });
});

test('an expired borrowed ChatGPT login is unavailable with a reason, not an exception', async t => {
  const { state } = await isolatedHome(t, { codexAuth: { tokens: { access_token: unsignedJwt(1), account_id: 'acct' } } });
  const credentials = fileCredentials(state);
  const codex = (await discoverAccounts({ credentials, env: {} })).find(account => account.id === 'openai-codex');
  assert.equal(codex.state, 'unavailable');
  assert.match(codex.reason, /renewal|login/i);
});

test('stored credentials win and are reported as Dojo-owned', async t => {
  const { state } = await isolatedHome(t);
  const credentials = fileCredentials(state);
  await credentials.modify('opencode', async () => ({ type: 'api_key', key: secret }));
  const zen = (await discoverAccounts({ credentials, env: {} })).find(account => account.id === 'opencode');
  assert.deepEqual({ state: zen.state, source: zen.source }, { state: 'available', source: 'dojo' });
});

const model = (id, extra = {}) => ({ id, name: id, type: 'chat', input: ['text', 'image'], api: 'openai-completions',
  contextWindow: 100000, maxTokens: 8000, cost: { input: 1, output: 2 }, ...extra });
const providers = {
  'opencode-go': { getAllModels: async () => [model('deepseek-v4.1-flash', { cost: { input: 0.15, output: 0.6 } }), model('text-only', { input: ['text'] }),
    model('promo-free', { cost: { input: 0, output: 0 } }), model('deepseek-v4.1-flash-free', { cost: { input: 0, output: 0 } }), model('embed', { type: 'embedding' })] },
  'openai-codex': { getAllModels: async () => [model('gpt-6-luna', { api: 'openai-codex-responses', cost: { input: 0, output: 0 } }), model('gpt-6.1-sol')] },
};
const available = id => ({ id, label: id, state: 'available', source: 'dojo' });

test('the catalog lists chat models with image input, without free-tier OpenCode models', async () => {
  const rows = await listModels([available('opencode-go'), { id: 'opencode', label: 'Zen', state: 'unavailable', reason: 'x' }, available('openai-codex')],
    { providerFor: id => providers[id] });
  assert.deepEqual(rows.map(row => `${row.account}/${row.model}`),
    ['opencode-go/deepseek-v4.1-flash', 'openai-codex/gpt-6-luna', 'openai-codex/gpt-6.1-sol']);
  // Subscription accounts report zero cost; that is not a free tier.
  assert.equal(rows[1].cost.input, 0);
});

test('default selection follows account preference and pinned models', async () => {
  const rows = await listModels([available('openai-codex'), available('opencode-go')], { providerFor: id => providers[id] });
  assert.deepEqual(defaultSelection(rows), { account: 'opencode-go', model: 'deepseek-v4.1-flash' });
  assert.deepEqual(defaultSelection(rows.filter(row => row.account === 'openai-codex')), { account: 'openai-codex', model: 'gpt-6-luna' });
  assert.equal(defaultSelection([]), null);
});

test('default selection falls back to the cheapest model when the pin is not listed', () => {
  const rows = [{ account: 'opencode-go', model: 'big', cost: { input: 3, output: 9 }, context: 1 },
    { account: 'opencode-go', model: 'small', cost: { input: 0.1, output: 0.4 }, context: 1 }];
  assert.deepEqual(defaultSelection(rows), { account: 'opencode-go', model: 'small' });
});

test('the summary model is the account\'s pinned default when cheaper than the selection, never another cheap model', () => {
  const rows = [{ account: 'opencode-go', model: 'kimi-k3', cost: { input: 3, output: 15 } },
    { account: 'opencode-go', model: 'deepseek-v4.1-flash', cost: { input: 0.15, output: 0.6 } },
    { account: 'opencode-go', model: 'unreleased-cheap', cost: { input: 0.01, output: 0.02 } },
    { account: 'b', model: 'other', cost: { input: 0, output: 0 } }];
  assert.equal(summaryModel(rows, { account: 'opencode-go', model: 'kimi-k3' }), 'deepseek-v4.1-flash');
  assert.equal(summaryModel(rows, { account: 'opencode-go', model: 'unreleased-cheap' }), 'unreleased-cheap');
  assert.equal(summaryModel(rows, { account: 'opencode-go', model: 'deepseek-v4.1-flash' }), 'deepseek-v4.1-flash');
  assert.equal(summaryModel(rows, { account: 'c', model: 'x' }), 'x');
});

test('OpenCode models that train on request data are not offered', async () => {
  const model = (id, extra = {}) => ({ id, name: id, type: 'chat', input: ['text', 'image'], cost: { input: 1, output: 2 }, ...extra });
  const rows = await listModels([{ id: 'opencode-go', label: 'OpenCode Go', state: 'available' }],
    { providerFor: () => ({ getAllModels: async () => [model('muse-spark-1.2-contributor'), model('glm-5.3-flash')] }) });
  assert.deepEqual(rows.map(row => row.model), ['glm-5.3-flash']);
});

test('cache handling follows the request API, and unknown shapes pass through unmarked', () => {
  assert.equal(cacheShape('anthropic-messages'), 'anthropic');
  assert.equal(cacheShape('openai-codex-responses'), 'responses');
  assert.equal(cacheShape('openai-completions'), 'passthrough');
  assert.equal(cacheShape('google-generative-ai'), 'passthrough');
  const original = { model: 'm', messages: [{ role: 'user', content: '<chat>\n1+1|a\n2+1|b\n3+1|c\n4+1|d\n</chat>' }], cache_control: { type: 'x' } };
  const out = cachePayload('openai-completions', original, { cacheId: 'dojo' });
  assert.equal(out.prompt_cache_key, undefined);
  assert.equal(out.cache_control, undefined);
  assert.equal(out.messages[0].content, original.messages[0].content);
});

test('a non-first-party Anthropic-API model uses short retention and no subscription identity', async () => {
  let seen;
  const base = { id: 'opencode-go', streamSimple(model, context, options) {
    const result = (async () => { seen = { options, payload: await options.onPayload({ system: 's', messages: [{ role: 'user', content: '<chat>\n1+1|a\n2+1|b\n3+1|c\n4+1|d\n</chat>' }], tools: [{ name: 't' }] }) }; return {}; })();
    return { result: () => result };
  } };
  await wrapCacheProvider(base, { cacheId: 'dojo', claudeSubscription: true }).streamSimple({ id: 'claude-haiku-5-5', api: 'anthropic-messages' }, {}).result();
  assert.equal(seen.options.cacheRetention, 'short');
  assert.equal(seen.options.sessionId, 'dojo');
  assert.equal(seen.payload.tools.at(-1).cache_control.ttl, '5m');
});

test('createDojoModels registers every usable account and skips the rest', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const models = await createDojoModels({ stateDir: state, providers: ['opencode-go', 'opencode', 'openai-codex'], store: fileCredentials(state), env: {} });
  assert.deepEqual(models.dojoAccounts, ['opencode-go']);
  await assert.rejects(createDojoModels({ stateDir: state, providers: ['opencode', 'openai-codex'], store: fileCredentials(state), env: {} }), /Sign in to use Dojo/);
});

test('an explicit single provider keeps its original error when it has no login', async t => {
  const { state } = await isolatedHome(t);
  await assert.rejects(createDojoModels({ stateDir: state, provider: 'openai-codex', store: fileCredentials(state), env: {} }), /Codex/);
});

test('prepareAgentModels starts on the preferred account and reports unavailable ones', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const result = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {}, cacheId: 'dojo:test' });
  assert.deepEqual(result.selection, { account: 'opencode-go', model: 'deepseek-v4.1-flash' });
  assert.equal(result.accounts.find(account => account.id === 'opencode-go').state, 'available');
  assert.equal(result.accounts.find(account => account.id === 'opencode-go').backend, 'file');
  assert.equal(result.accounts.find(account => account.id === 'opencode-go').source, 'dojo');
  assert.equal(result.accounts.find(account => account.id === 'openai-codex').state, 'unavailable');
  assert.ok(result.rows.every(row => row.account === 'opencode-go'));
  assert.ok(!JSON.stringify({ accounts: result.accounts, rows: result.rows }).includes(secret));
});

test('an explicit provider and model are honoured, and fail visibly without a login', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const result = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {}, config: { provider: 'opencode-go', model: 'kimi-k3' } });
  assert.deepEqual(result.selection, { account: 'opencode-go', model: 'kimi-k3' });
  await assert.rejects(prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {}, config: { provider: 'openai-codex', model: 'gpt-6-luna' } }),
    /openai-codex account has no usable login/);
});

test('a legacy ChatGPT model keeps applying when ChatGPT is the selected account', async t => {
  const { state } = await isolatedHome(t, { codexAuth: { tokens: { access_token: unsignedJwt(Math.floor(Date.now() / 1000) + 3600), account_id: 'acct' } } });
  const result = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {}, config: { legacyModel: 'gpt-6.1-sol' } });
  assert.deepEqual(result.selection, { account: 'openai-codex', model: 'gpt-6.1-sol' });
});

test('a persisted account/model selection is restored and its summary model follows that account', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const rows = [model('expensive', { cost: { input: 1, output: 2 } }), model('deepseek-v4.1-flash', { cost: { input: 0.1, output: 0.2 } })];
  const result = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {},
    config: { modelSelection: { account: 'opencode-go', model: 'expensive' } }, providerFor: () => ({ getAllModels: async () => rows }) });
  assert.deepEqual(result.selection, { account: 'opencode-go', model: 'expensive' });
  assert.equal(result.selectionAvailable, true);
  assert.equal(result.summaryModel, 'deepseek-v4.1-flash');
});

test('an unknown saved selection is preserved as unavailable instead of rerouting to an available account', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const result = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {},
    config: { modelSelection: { account: 'removed-account', model: 'model-x' } } });
  assert.deepEqual(result.selection, { account: 'removed-account', model: 'model-x' });
  assert.equal(result.selectionAvailable, false);
  assert.match(result.selectionReason, /no longer listed|unavailable/i);
});

test('an imported key is seeded so requests resolve it from the Dojo store, not the environment', async t => {
  const { state } = await isolatedHome(t, { piAuth: { 'opencode-go': { type: 'api_key', key: secret } } });
  const { models } = await prepareAgentModels({ stateDir: state, credentials: fileCredentials(state), env: {}, cacheId: 'dojo:test' });
  const resolved = await models.getAuth('opencode-go');
  assert.equal(resolved.auth.apiKey, secret);
  assert.equal(resolved.source, 'stored credential');
  assert.equal(await models.getAuth('opencode'), undefined);
});

test('request auth ignores ambient OPENCODE_API_KEY even when the provider is registered', async t => {
  const { state } = await isolatedHome(t);
  const previous = process.env.OPENCODE_API_KEY;
  process.env.OPENCODE_API_KEY = 'ambient-should-not-be-used';
  t.after(() => { if (previous === undefined) delete process.env.OPENCODE_API_KEY; else process.env.OPENCODE_API_KEY = previous; });
  const models = await createDojoModels({ stateDir: state, provider: 'opencode', credentials: fileCredentials(state), env: {} });
  assert.equal(await models.getAuth('opencode'), undefined);
});

test('an expired imported subscription login is unavailable at discovery, matching what use would do', async t => {
  const past = Date.now() - 60_000;
  const { state } = await isolatedHome(t, { piAuth: { openai: { type: 'oauth', access: 'a', refresh: 'r', expires: past } } });
  const openai = (await discoverAccounts({ credentials: fileCredentials(state), env: {} })).find(account => account.id === 'openai');
  assert.equal(openai.state, 'unavailable');
  assert.match(openai.reason, /renewal/i);
});
