import { DojoCredentials, createDojoModels, subscriptionProvider } from './providers.mjs';

/** Accounts in default preference order. Each account is one credential source, keyed by pi-ai provider id. */
export const ACCOUNTS = [
  { id: 'opencode-go', label: 'OpenCode Go' },
  { id: 'opencode', label: 'OpenCode Zen' },
  { id: 'openai-codex', label: 'ChatGPT' },
  { id: 'openai', label: 'OpenAI' },
];
export const ACCOUNT_ORDER = ACCOUNTS.map(({ id }) => id);
const labels = new Map(ACCOUNTS.map(({ id, label }) => [id, label]));
const FREE_TIER_PROVIDERS = new Set(['opencode', 'opencode-go']);
const DEFAULT_MODELS = {
  'opencode-go': 'deepseek-v4.1-flash', opencode: 'deepseek-v4.1-flash',
  'openai-codex': 'gpt-6-luna', openai: 'gpt-6-luna',
};
// Providers kept for hand-written settings only; they are not offered as accounts.
const EXPLICIT_DEFAULTS = { anthropic: 'claude-haiku-5-5' };

/**
 * Reports which accounts could carry a request. Reads credentials and expiry only: no login, session, token
 * refresh or network call. The result never contains secret material.
 */
export async function discoverAccounts({ credentials, env = process.env } = {}) {
  const found = [];
  for (const { id, label } of ACCOUNTS) {
    let stored;
    try { stored = await credentials.read(id); }
    catch (error) { found.push({ id, label, state: 'unavailable', reason: error.message }); continue; }
    if (stored?.borrowedSubscription && stored.type === 'oauth') { found.push({ id, label, state: 'available', source: 'borrowed' }); continue; }
    if (['oauth', 'api_key'].includes(stored?.type)) { found.push({ id, label, state: 'available', source: 'dojo', backend: credentials.backend }); continue; }
    try {
      const candidate = await credentials.importCandidate(id, { env });
      found.push({ id, label, state: 'available', source: candidate.borrowedSubscription ? 'borrowed' : 'import' });
    } catch (error) {
      found.push({ id, label, state: 'unavailable', reason: error.message });
    }
  }
  return found;
}

// OpenCode "contributor" models train on request data, which a workspace's Privacy setting refuses by default.
const trainsOnRequests = model => /-contributor$/.test(model.id);
const isFreeTier = model => model.id.endsWith('-free') || ((model.cost?.input ?? 0) === 0 && (model.cost?.output ?? 0) === 0);

/**
 * Selectable models for the available accounts, in account preference order. Desktop control needs image input,
 * and OpenCode free-tier models are refused to direct clients, so they are not offered.
 */
export async function listModels(accounts, { providerFor = subscriptionProvider } = {}) {
  const rows = [];
  for (const account of accounts) {
    if (account.state !== 'available') continue;
    for (const model of await providerFor(account.id).getAllModels()) {
      if ((model.type ?? 'chat') !== 'chat' || !model.input?.includes('image')) continue;
      if (FREE_TIER_PROVIDERS.has(account.id) && (isFreeTier(model) || trainsOnRequests(model))) continue;
      rows.push({ account: account.id, accountLabel: account.label, model: model.id, name: model.name ?? model.id,
        api: model.api, context: model.contextWindow, maxTokens: model.maxTokens, reasoning: !!model.reasoning,
        cost: { input: model.cost?.input ?? 0, output: model.cost?.output ?? 0 } });
    }
  }
  return rows;
}

const price = row => row.cost.input + row.cost.output;

/** The account's pinned default when listed, else its cheapest model; accounts are tried in preference order. */
export function defaultSelection(rows, order = ACCOUNT_ORDER) {
  for (const account of order) {
    const own = rows.filter(row => row.account === account);
    if (!own.length) continue;
    const pinned = own.find(row => row.model === DEFAULT_MODELS[account]);
    const row = pinned ?? [...own].sort((a, b) => price(a) - price(b) || b.context - a.context)[0];
    return { account: row.account, model: row.model };
  }
  return null;
}

/**
 * The model that writes memory summaries: the account's pinned default when it costs less than the selected
 * model, else the selected model. A summary never uses an arbitrary cheapest model, since the cheapest listed
 * models can be unavailable (not yet released, or refused by the workspace's privacy settings).
 */
export function summaryModel(rows, selection) {
  const own = rows.filter(row => row.account === selection.account);
  const pinned = own.find(row => row.model === DEFAULT_MODELS[selection.account]);
  const chosen = own.find(row => row.model === selection.model);
  return pinned && chosen && price(pinned) < price(chosen) ? pinned.model : selection.model;
}

/**
 * Builds the model catalog for every usable account and picks the starting selection. An explicit provider and
 * model from settings is honoured as written, so a hand-written configuration keeps working.
 */
export async function prepareAgentModels({ stateDir, config = {}, cacheId, env = process.env, credentials = new DojoCredentials(stateDir), providerFor = subscriptionProvider } = {}) {
  const accounts = await discoverAccounts({ credentials, env });
  const explicitModel = config.model ?? EXPLICIT_DEFAULTS[config.provider];
  const explicit = config.provider && explicitModel ? { account: config.provider, model: explicitModel } : null;
  const usable = accounts.filter(account => account.state === 'available').map(account => account.id);
  const providers = [...new Set([...(config.provider ? [config.provider] : []), ...usable])];
  const models = await createDojoModels({ stateDir, providers, store: credentials, cacheId, onPayload: config.onPayload, env });
  if (explicit && !models.dojoAccounts.includes(explicit.account)) {
    throw new Error(`The configured ${explicit.account} account has no usable login.`);
  }
  const available = accounts.map(account => models.dojoAccounts.includes(account.id)
    ? account.source === 'borrowed' ? account : { ...account, source: 'dojo', backend: credentials.backend }
    : { ...account, state: 'unavailable', reason: account.reason ?? 'Not signed in.' });
  const rows = await listModels(available, { providerFor });
  let selection = explicit ?? (config.modelSelection ?? null);
  if (selection && typeof selection.account !== 'string') selection = null;
  if (!selection) selection = config.provider ? defaultSelection(rows, [config.provider]) : defaultSelection(rows);
  // A model chosen before accounts existed keeps applying to the ChatGPT account it was chosen for.
  if (!explicit && !config.modelSelection && selection?.account === 'openai-codex' && config.legacyModel
    && rows.some(row => row.account === 'openai-codex' && row.model === config.legacyModel)) {
    selection = { account: 'openai-codex', model: config.legacyModel };
  }
  const selectionAvailable = !!selection && rows.some(row => row.account === selection.account && row.model === selection.model);
  if (explicit && !selectionAvailable) throw new Error(`The configured ${explicit.account}/${explicit.model} model is not available.`);
  const selectedForSummary = selectionAvailable ? selection : defaultSelection(rows);
  const selectionReason = selectionAvailable ? undefined
    : accounts.find(account => account.id === selection?.account)?.reason ?? (selection ? 'The saved model is no longer listed.' : 'Sign in to use Dojo.');
  return { models, accounts: available, rows, selection, selectionAvailable, selectionReason, credentials,
    summaryModel: selectedForSummary ? summaryModel(rows, selectedForSummary) : undefined };
}
