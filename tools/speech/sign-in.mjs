import { createModels } from '@earendil-works/pi-ai/models';
import { subscriptionProvider } from './providers.mjs';

const openCodeAccounts = new Set(['opencode', 'opencode-go']);
const noAmbientAuth = { env: async () => undefined, fileExists: async () => false };

/** Confirms an OpenCode key against that account's provider model endpoint without persisting it first. */
export async function validateOpenCodeKey(account, key, { createModelCatalog = createModels, providerFor = subscriptionProvider } = {}) {
  if (!openCodeAccounts.has(account)) throw new Error('Choose OpenCode Go or OpenCode Zen.');
  if (typeof key !== 'string' || !key.trim() || key.length > 4096) throw new Error('Enter a valid OpenCode API key.');
  const credential = { type: 'api_key', key: key.trim() };
  const credentials = { read: async id => id === account ? credential : undefined,
    modify: async (_id, change) => change(undefined), delete: async () => {} };
  const models = createModelCatalog({ credentials, authContext: noAmbientAuth });
  models.setProvider(providerFor(account));
  const refreshed = await models.refresh({ providers: [account], force: true });
  const error = refreshed.errors.get(account);
  if (error) throw new Error(error.message.replaceAll(key, '[redacted]'));
  const listed = models.getAllModels(account);
  if (!listed.length) throw new Error('OpenCode returned no models for this key.');
  return { models: listed.length };
}
