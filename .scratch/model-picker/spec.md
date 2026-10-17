# Model picker, sign-in and onboarding

Add a model picker to the desktop shell and make the voice agent's model and
account selection explicit. Today the runtime hardcodes one provider
(`openai-codex` by default) and borrows its login from another app. The user no
longer has a ChatGPT subscription and keeps an OpenCode subscription; one, several
or all supported accounts may be present.

## Experience

A generic agent glyph in the top bar opens a native menu, following the existing
panel conventions (theme, font, shape, keyboard, per-display lifecycle). The menu
lists the models the agent can use right now, grouped by account, with the current
selection marked. Choosing a model applies it to new voice requests globally. A
request already running finishes on the model it started with.

Only models an available account can serve are selectable. A missing, expired or
unreadable login removes that account's models from the list, and the picker
explains why an account is absent in a secondary line rather than as a selectable
row. The current selection is the one exception: if its account becomes
unavailable it stays visible, marked unavailable with the reason, so the user is
never shown a model name that silently changed. Failures after selection (an
expired token mid-session, a quota or outage) surface visibly, name the account,
and send the request nowhere else. There is no automatic reroute to another
account.

Only models that accept image input are listed, because desktop control depends
on screen captures.

## Accounts

An **account** is one credential source that can carry model requests. Supported
accounts are OpenAI subscription (`openai-codex`), OpenAI API (`openai`), OpenCode
Go (`opencode-go`) and OpenCode Zen (`opencode`). Each is discovered independently,
so any subset works. All are direct accounts called through pi-ai inside
pi-durable.

Claude is not a supported account for Dojo's agent. Dojo adds no Claude login,
CLI runtime or credential borrowing. Claude models that an OpenCode account
happens to serve are ordinary OpenCode models and appear like any other.

Routing keys on the account, never on the provider kind. A saved selection is an
account id plus a model id. If that account disappears, Dojo does not look for
another account of the same kind; the selection is reported unavailable. Unknown
account ids in saved state parse and round-trip as unavailable rather than
failing startup. Each account's credentials stay separate: nothing is read from
ambient environment variables such as `OPENCODE_API_KEY` unless the account was
explicitly configured to use them.

OpenCode accounts authenticate with an API key, not OAuth. The existing rule
"no API-key fallback" is narrowed, not dropped: an OpenCode key is used only when
the user imported or entered it for Dojo, and an API key is never substituted for
a missing subscription login. Keys are stored with the existing private
credential file and never appear in arguments, logs or the picker.

When one model is reachable through several accounts, the first account in the
user's ordered preference carries it. The default order puts OpenCode Go first,
then OpenCode Zen, then ChatGPT, then OpenAI. The picker shows the account that would carry each
model so the choice is never hidden.

## Credential storage

Dojo stores its own account credentials in the system password store through the
freedesktop Secret Service (GNOME Keyring, already configured by `home-manager.nix`),
not in a plaintext file. Each account is one item with attributes `service=dojo` and
`account=<account id>` whose secret is the serialized credential (an API key, or an
OAuth access and refresh token, expiry and account id).

The service talks to it with `secret-tool`: `store` takes the secret on standard
input, and `lookup` returns it on standard output, so secrets are never in
arguments. Only the non-secret attributes appear in argv. Writes go through the
existing serialized credential chain so refresh and sign-in cannot interleave.

Lookups are cached in memory for a short time, and every call has a timeout.
Discovery never triggers an unlock prompt: it checks the collection's `Locked`
state first and reports "keyring is locked" as the account's reason instead of
waiting on a dialog.

If no Secret Service is available (a headless session), Dojo falls back to the
private 0600 credential file and the account row says so, so the downgrade is
visible rather than silent. Existing entries in `credentials.json` migrate once:
write to the keyring, read back to verify, then remove the file entry.

Borrowed logins (Codex's `auth.json`) are read-only snapshots and are not copied
into the keyring.

## Sign-in and onboarding

Users sign in from the shell, to ChatGPT, to OpenCode, or both, and use either.
Nothing requires a terminal or stopping the voice service.

**First run.** With no usable account, voice requests fail visibly with "Sign in to
use Dojo" instead of a provider error, the top-bar glyph shows a needs-setup state,
and opening it shows onboarding rather than an empty model list. Onboarding needs
at least one account and offers both.

**Account rows.** The menu lists each account with its state (signed in, not signed
in, needs renewal) and a Sign in or Sign out action. Sign out deletes only that
account's Dojo credential and never touches another app's login.

**ChatGPT.** The service runs pi-ai's OpenAI login and reports progress to the
panel. The default method is device code: the panel shows the code and
verification address and waits, cancellably. Browser login is the alternative; it
needs port 1455, which Codex CLI shares, so the panel falls back to a pasted
redirect URL. The result is an independent Dojo login that Dojo refreshes. An
existing Codex login can still be used read-only as a separate, explicit option.

**OpenCode.** The user pastes an API key into a masked native editor, following the
Wi-Fi password editor. The service validates it with a model-list request before
saving, stores it in the system password store, and the key never appears in arguments,
logs, the picker or any response. An explicit developer path can import
a key from `~/.pi/agent/auth.json` or a secrets manager such as fnox with
1Password; it is not part of onboarding.

**Secrets across the socket.** Sign-in commands are write-only. A key crosses the
private, owner-only voice socket once and is never returned. `models` and status
responses carry state and reason only. The shell keeps a typed key in the editor
buffer only until submission or dismissal.

The service owns login flows and writes through the existing serialized
credential store, which replaces `tools/speech/login.mjs`'s requirement to stop
the service. That script remains for headless use.

## Defaults

With no saved selection, the default is the first available account's cheapest
image-capable model in preference order. Price and context are shown per row.

`deepseek-v4.1-flash-free` is not usable as a default through a direct provider
call. It is live on OpenCode Zen and appears in `GET /zen/v1/models` for a valid
key, but a direct completion is refused with `403 FreeTierError`: "OpenCode's free
tier can only be used from within OpenCode." OpenCode's own stateless
`/api/experimental/generate` endpoint is refused the same way. Dojo must not
imitate OpenCode's client identity to get around that restriction.

A real OpenCode session (`opencode run -m opencode/deepseek-v4.1-flash-free`)
passes that check but is then refused by the workspace's Privacy setting: "Free
models may train on request data. Allow free endpoints in your workspace's Privacy
settings to use them." Dojo requests carry desktop screenshots and memory, so
enabling that setting is a privacy decision for the user, not a default. Free-tier
models are excluded from the picker. Running requests through the OpenCode CLI or
service as an execution runtime was considered and rejected: it would bypass
pi-durable memory and the shared desktop rules. Only direct accounts are in scope.

The paid `deepseek-v4.1-flash` is the intended low-cost default: $0.30/$1.20 per
million tokens on Zen with a 1M context, or $0.15/$0.60 on Go within a
subscription.

## Wiring

Provider construction becomes per-account rather than a single global provider.
The cache wrapper currently branches on provider id, treating everything that is
not Anthropic as the OpenAI Responses shape. OpenCode serves models over several
protocols, so cache handling must key on the model's API, and unsupported shapes
must skip cache marks rather than corrupt the payload.

The summary model for memory is derived from the selected account: the cheapest
listed small model it offers, else the selected model. The voice service exposes list and
select commands over its existing socket; the shell never reads credentials.

## Out of scope

Per-conversation model choice, automatic fallback between accounts, and usage or
quota display. Claude, and running requests through the OpenCode runtime, are also
out of scope.

## Lessons from T3 Code

T3 Code (`pingdotgg/t3code`, `docs/internals/providers.md` and
`packages/contracts/src/providerInstance.ts`) already solves the same routing
problem, and this spec follows it where it fits:

- **Driver kind versus instance.** A driver kind names an implementation; an
  instance is one configured account and is the only routing key. Dojo's
  "account" is that instance.
- **Unknown or removed instances degrade, never crash.** Persisted selections that
  reference an instance this build cannot serve are marked unavailable, and the
  picker keeps the selected model's label.
- **Setup is never a health-check side effect.** Discovery reads credentials and
  expiry only. It does not open a session, start a login, or refresh a token.
  Catalog refresh is explicit, and a cached list does not prove current access.
- **Never inherit credentials an instance was not configured with.**
- **One owner refreshes a token.** Dojo already refuses to rotate another app's
  refresh token and keeps that rule.
- **Separate catalog from capability.** Bundled model data supports offline start;
  an authoritative empty catalog clears a stale list.

T3 differs in one respect that matters here: it wraps each provider's CLI and owns
no credentials, while Dojo calls providers directly and therefore holds them.

## Resolved questions

- **Claude.** Not supported for Dojo's agent. Existing `anthropic` provider code in
  `tools/speech/providers.mjs` is left alone by this feature; whether to remove it
  is a separate decision.
- **Free DeepSeek 4.1.** Live and listed, but refused to direct clients with
  `FreeTierError` (see Defaults). It is not a candidate default.
- **OpenCode key.** The single key at `op://Personal/Opencode/credential` is the
  one Dojo uses. It authenticates against Zen (`/zen/v1/models`, 49 models) and
  carries a real `deepseek-v4.1-flash` completion on OpenCode Go (verified through
  pi-ai's `opencode-go` provider). Zen has no credits, so Go is the working
  account. Go requires an `x-opencode-session` header, which pi-ai sets from the
  `sessionId` option; Dojo's cache wrapper already passes one.
- **Packages.** pi-ai, pi-durable and chord are upgraded together to 1.3.0 in
  `tools/speech`. pi-durable 1.2.0 renamed a tool result's `content` to `output`;
  Dojo's tools were migrated and the speech suite passes.

## Open questions

- The Zen account has no credits: a paid `deepseek-v4.1-flash` request through
  OpenCode returned "Insufficient account funds". Paid Zen models need a balance.
- How the secret reaches the runtime. The key is available through 1Password
  (fnox with the `op` provider); the voice service needs a launch path that
  resolves it without writing it to disk, arguments or logs.
