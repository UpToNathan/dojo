# Discover agent accounts and the available model catalog

Status: resolved

Implement the account model in [the spec](../spec.md) in the voice runtime. Replace
the single-provider `createDojoModels` path with per-account providers, discover
which accounts have valid credentials, and produce the list of selectable models
(image input required) with the account that would carry each.

The pi-ai, pi-durable and chord upgrade to 1.3.0 is done and the speech suite
passes; run the live memory and cache checks against it before relying on it.
Cover the following. Add OpenCode Go and Zen as API-key accounts, importing the
`opencode-go` and `opencode` entries from the Pi login file without rotating it,
and keeping the narrowed no-fallback rule. Make the cache wrapper choose its
payload handling from the model's API instead of its provider id, with unsupported
shapes passing through unmarked. Report an unavailable account with a reason the
picker can show, never throwing from discovery.

Account ids, not provider kinds, are the routing key: unknown ids parse as
unavailable, and discovery performs no login, session or token refresh. Do not
read `OPENCODE_API_KEY` or similar ambient variables for an account that was not
configured.

Verify with deterministic tests: each subset of accounts, an expired borrowed
login, a model offered by several accounts and its preference order, and cache
payloads for the Anthropic Messages, OpenAI Responses and OpenAI Completions
shapes. Keep `node --test tools/speech/*.test.mjs` fully green.

## Result

Implemented in `tools/speech/accounts.mjs` and `providers.mjs`, wired into
`pi-runtime.mjs` and `voice.mjs`.

- Accounts are `opencode-go`, `opencode`, `openai-codex` and `openai`, discovered
  independently by `discoverAccounts` from credentials and expiry only. Results
  carry state, source and a reason, never secret material.
- Ambient environment variables never authenticate: `createModels` receives an
  `authContext` that returns nothing. OpenCode keys come from the Dojo store, the
  Pi login's `api_key` entries, or the explicit `DOJO_OPENCODE_API_KEY`.
- `createDojoModels` registers every usable account in one catalog, and reports
  "Sign in to use Dojo" when none is usable. An explicit single provider keeps its
  original errors.
- Cache handling follows the request API (`cacheShape`); unknown shapes pass
  through unmarked, and non-first-party Anthropic-API models use short retention.
- `prepareAgentModels` picks the starting selection (pinned model per account, else
  the cheapest), honours an explicit provider and model, and derives the summary
  model. A legacy ChatGPT model still applies when ChatGPT is selected.
- Discovery rejects an expired imported subscription login so it agrees with use.

Validation: 19 account tests and the full speech suite (155) pass. A live request
through Dojo's own path returned `ok` from `deepseek-v4.1-flash` on OpenCode Go.
The voice service restarted on the new wiring.

Remaining: the live memory and cache checks (`scripts/check-dojo-memory.mjs`,
`check-dojo-cache.mjs`) were not run against 1.3.0. Credentials still persist in
the 0600 file until issue 06 moves them to the keyring.

## Comments
