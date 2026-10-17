# Run sign-in and sign-out flows inside the voice service

Status: resolved
Blocked by: 01, 02, 06

Implement the service half of sign-in in [the spec](../spec.md). Add write-only
socket commands to `tools/speech/voice.mjs`: begin and cancel a ChatGPT login,
submit an OpenCode key, and sign out of an account. Progress events (device code and
verification address, browser URL, paste prompt, completion, failure) reach the
shell through the existing event path.

Run pi-ai's OpenAI login in the service with device code as the default and browser
login as the alternative, handling the port 1455 conflict with Codex CLI by falling
back to a pasted redirect URL. Store the result through the keyring-backed credential store (issue 06) and its
serialized chain so no restart or lock is needed, and make Dojo the refresher of that login.
Validate an OpenCode key with a model-list request before saving it. Never log,
return or echo a key, and keep sign-out scoped to one account.

Report the needs-setup state when no account is usable, so a voice request fails with
a clear message. Verify with fake OAuth and key-validation endpoints: cancellation,
expiry, a taken port, a rejected key, concurrent writes, file mode, and that no
response or log contains secret material.

## Result

Implemented the service-side write-only commands in `tools/speech/voice.mjs`, with
key validation in `sign-in.mjs` and credential persistence through issue 06's
serialized store.

- ChatGPT login supports cancel, defaults pi-ai's method selector to device code,
  and allows the browser flow. Login events (verification link/code, paste prompt,
  completion/failure) go to the shell's existing response-event path. Pasted callback
  input is accepted only by the write-only `login-input` command.
- OpenCode keys are validated by refreshing that account's model catalog before
  saving. The key is sent only in the socket request, stored through DojoCredentials,
  and redacted from errors. Sign-out deletes only the requested Dojo account and
  cancels a matching in-progress login.
- The catalog refreshes on login/sign-out; no-account requests fail clearly.

Validation: fake OAuth and key-validation paths cover default device selection,
explicit cancellation, rejected keys, non-disclosure, and account-scoped sign-out.
The full speech suite passes: 178 passed, 1 gated live-keyring check skipped. No real
OAuth or provider endpoint was contacted. The native picker does not yet expose sign-in/sign-out buttons or a text editor
for the browser redirect prompt; those remain onboarding/account-UI follow-ups.

## Comments
