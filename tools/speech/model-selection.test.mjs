import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, mkdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
const testRoot = await mkdtemp(join(tmpdir(), 'dojo-selection-suite-'));
const previousStateHome = process.env.XDG_STATE_HOME, previousRuntime = process.env.XDG_RUNTIME_DIR;
process.env.XDG_STATE_HOME = testRoot;
process.env.XDG_RUNTIME_DIR = testRoot;
await mkdir(testRoot, { recursive: true });
const { VoiceService } = await import('./voice.mjs');
after(async () => {
  if (previousStateHome === undefined) delete process.env.XDG_STATE_HOME; else process.env.XDG_STATE_HOME = previousStateHome;
  if (previousRuntime === undefined) delete process.env.XDG_RUNTIME_DIR; else process.env.XDG_RUNTIME_DIR = previousRuntime;
  await rm(testRoot, { recursive: true, force: true });
});

const row = (account, model, extra = {}) => ({ account, accountLabel: account, model, name: model, api: 'openai-responses',
  context: 100000, maxTokens: 8000, reasoning: false, cost: { input: 1, output: 2 }, ...extra });

test('models and select-model expose only public catalog metadata and persist the chosen account/model', async t => {
  const service = new VoiceService();
  service.config = { pi: { instructions: 'unchanged' } };
  service.state = { chats: [], activeThreadId: null };
  const secret = 'do-not-expose-key';
  service.runtime = {
    accounts: [{ id: 'opencode-go', label: 'OpenCode Go', state: 'available', source: 'dojo', backend: 'keyring' },
      { id: 'openai', label: 'OpenAI', state: 'unavailable', reason: 'Not signed in.' }],
    modelRows: [row('opencode-go', 'deepseek-v4.1-flash')],
    selection: { account: 'opencode-go', model: 'deepseek-v4.1-flash' }, selectionAvailable: true,
    setModelSelection: async (selection, options) => { service.runtime.selection = selection; service.runtime.selectionAvailable = options.available; },
  };
  const configFile = join(testRoot, 'dojo/voice/config.json');
  const catalog = await service.handle({ action: 'models' });
  assert.equal(catalog.models.length, 1);
  assert.equal(catalog.accounts[0].backend, 'keyring');
  assert.doesNotMatch(JSON.stringify(catalog), new RegExp(secret));
  await assert.rejects(service.handle({ action: 'select-model', account: 'openai', model: 'stale-model' }), /not currently available/);
  await service.handle({ action: 'select-model', account: 'opencode-go', model: 'deepseek-v4.1-flash' });
  const saved = JSON.parse(await readFile(configFile, 'utf8'));
  assert.deepEqual(saved.modelSelection, { account: 'opencode-go', model: 'deepseek-v4.1-flash', name: 'deepseek-v4.1-flash' });
  assert.equal(saved.pi.instructions, 'unchanged');
});

test('OpenCode key submission validates before saving and never echoes the key', async () => {
  const service = new VoiceService();
  service.state = { chats: [], activeThreadId: null };
  service.ui = async () => {};
  let stored, validated, deleted;
  service.validateOpenCodeKey = async (account, key) => { validated = { account, key }; };
  service.runtime = { credentialStore: { modify: async (account, change) => { stored = { account, value: await change(undefined) }; },
      delete: async account => { deleted = account; } },
    reloadAccountCatalog: async () => {}, accounts: [], modelRows: [], selection: null };
  const key = 'never-return-this-key';
  const response = await service.handle({ action: 'submit-opencode-key', account: 'opencode-go', key });
  assert.deepEqual(validated, { account: 'opencode-go', key });
  assert.deepEqual(stored, { account: 'opencode-go', value: { type: 'api_key', key } });
  assert.deepEqual(response, { saved: true, account: 'opencode-go' });
  assert.doesNotMatch(JSON.stringify(response), new RegExp(key));
  await service.handle({ action: 'sign-out', account: 'opencode-go' });
  assert.equal(deleted, 'opencode-go');
});

test('ChatGPT login defaults to device code, reports public progress and saves through the serialized store', async () => {
  const service = new VoiceService();
  service.state = { chats: [], activeThreadId: null };
  service.ui = async () => {};
  const persisted = [];
  service.runtime = { credentialStore: { modify: async (account, change) => persisted.push([account, await change(undefined)]) },
    reloadAccountCatalog: async () => {} };
  let method;
  service.oauthProvider = () => ({ auth: { oauth: { login: async (interaction) => {
    method = await interaction.prompt({ type: 'select', message: 'method' });
    interaction.notify({ type: 'device_code', userCode: 'ABCD', verificationUri: 'https://example.test' });
    return { type: 'oauth', access: 'private-access', refresh: 'private-refresh', expires: Date.now() + 3600_000 };
  } } } });
  const { loginId } = await service.handle({ action: 'begin-chatgpt-login' });
  await service.loginFlows.get(loginId).run;
  assert.equal(method, 'device_code');
  assert.equal(persisted[0][0], 'openai-codex');
  assert.equal(service.loginStatus.type, 'complete');
  assert.doesNotMatch(JSON.stringify(service.loginStatus), /private-access|private-refresh/);
});

test('login-status reports the device code while a login waits and never a secret', async () => {
  const service = new VoiceService();
  service.state = { chats: [], activeThreadId: null };
  service.runtime = { credentialStore: { modify: async () => {}, delete: async () => {} }, reloadAccountCatalog: async () => {} };
  assert.deepEqual(await service.handle({ action: 'login-status' }), { status: 'idle', active: false });
  service.oauthProvider = () => ({ auth: { oauth: { login: async interaction => {
    await interaction.prompt({ type: 'select', message: 'method' });
    interaction.notify({ type: 'device_code', userCode: 'WXYZ-1234', verificationUri: 'https://auth.example.test/device' });
    interaction.notify({ type: 'progress', message: 'Waiting' });
    return new Promise((_resolve, reject) => interaction.signal.addEventListener('abort', () => reject(new Error('aborted'))));
  } } } });
  const { loginId } = await service.handle({ action: 'begin-chatgpt-login' });
  await new Promise(resolve => setTimeout(resolve, 20));
  const progress = await service.handle({ action: 'login-status' });
  assert.deepEqual(progress, { status: 'device_code', active: true, loginId, userCode: 'WXYZ-1234', verificationUri: 'https://auth.example.test/device' });
  await service.handle({ action: 'cancel-login', loginId });
  await service.loginFlows.get(loginId)?.run;
  const done = await service.handle({ action: 'login-status' });
  assert.equal(done.status, 'failure');
  assert.equal(done.active, false);
});

test('ChatGPT login cancellation rejects the pending prompt and never stores a credential', async () => {
  const service = new VoiceService();
  service.state = { chats: [], activeThreadId: null };
  service.ui = async () => {};
  let writes = 0;
  service.runtime = { credentialStore: { modify: async () => { writes++; } }, reloadAccountCatalog: async () => {} };
  service.oauthProvider = () => ({ auth: { oauth: { login: async interaction => interaction.prompt({
    type: 'manual_code', message: 'Paste callback', placeholder: 'http://127.0.0.1:1455/auth/callback' }) } } });
  const { loginId } = await service.handle({ action: 'begin-chatgpt-login' });
  const flow = service.loginFlows.get(loginId);
  for (let i = 0; i < 100 && !flow.prompt; i++) await new Promise(resolve => setTimeout(resolve, 5));
  assert.ok(flow.prompt, `expected pending prompt, got ${service.loginStatus?.status}: ${service.loginStatus?.message ?? ''}`);
  assert.deepEqual(await service.handle({ action: 'cancel-login', loginId }), { cancelled: true });
  await flow.run;
  assert.equal(writes, 0);
  assert.equal(service.loginFlows.has(loginId), false);
});

test('the selected model remains visible with its reason when its account disappears', async () => {
  const service = new VoiceService();
  service.runtime = { accounts: [{ id: 'openai-codex', label: 'ChatGPT', state: 'unavailable', reason: 'Needs renewal.' }],
    modelRows: [], selection: { account: 'openai-codex', model: 'gpt-6-luna' }, selectionAvailable: false, selectionReason: 'Needs renewal.' };
  const catalog = await service.handle({ action: 'models' });
  assert.deepEqual(catalog.selection, { account: 'openai-codex', model: 'gpt-6-luna', name: 'gpt-6-luna', available: false, reason: 'Needs renewal.' });
  assert.deepEqual(catalog.models, []);
});
