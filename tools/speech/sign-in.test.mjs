import test from 'node:test';
import assert from 'node:assert/strict';
import { validateOpenCodeKey } from './sign-in.mjs';

const provider = id => ({ id, getModels: () => [], getAllModels: () => [] });

test('OpenCode key validation uses only the account credential and checks its model endpoint', async () => {
  let configured;
  const result = await validateOpenCodeKey('opencode-go', 'key-is-private', {
    providerFor: provider,
    createModelCatalog: options => {
      configured = options;
      return { setProvider(value) { assert.equal(value.id, 'opencode-go'); },
        async refresh(options) { assert.deepEqual(options, { providers: ['opencode-go'], force: true }); return { errors: new Map() }; },
        getAllModels() { return [{ id: 'model-a' }]; } };
    },
  });
  assert.deepEqual(result, { models: 1 });
  assert.equal((await configured.credentials.read('opencode-go')).key, 'key-is-private');
  assert.equal(await configured.credentials.read('opencode'), undefined);
  assert.equal(await configured.authContext.env('OPENCODE_API_KEY'), undefined);
});

test('invalid key errors are redacted and empty catalogs are rejected', async () => {
  const key = 'super-secret-token';
  await assert.rejects(validateOpenCodeKey('opencode', key, {
    providerFor: provider,
    createModelCatalog: () => ({ setProvider() {}, async refresh() { return { errors: new Map([['opencode', new Error(`rejected ${key}`)]]) }; } }),
  }), error => !error.message.includes(key) && /redacted/.test(error.message));
  await assert.rejects(validateOpenCodeKey('opencode', key, {
    providerFor: provider,
    createModelCatalog: () => ({ setProvider() {}, async refresh() { return { errors: new Map() }; }, getAllModels() { return []; } }),
  }), /no models/);
  await assert.rejects(validateOpenCodeKey('anthropic', key), /Choose OpenCode/);
});
