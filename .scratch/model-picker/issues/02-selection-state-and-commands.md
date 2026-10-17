# Persist the model selection and expose list and select commands

Status: resolved
Blocked by: 01

Store one global selection (account and model) in Dojo voice state and apply it to
new requests only. Add `models` and `select-model` commands to the voice socket in
`tools/speech/voice.mjs`, alongside the existing `status` and `history` commands.

`models` returns the catalog from issue 01 with the current selection, the
account that carries it, and unavailable accounts with their reasons.
`select-model` rejects a model that is not currently listed. Derive the summary
model from the selected account as described in the spec, replacing the hardcoded
Haiku and `gpt-6-luna` names in `voice.mjs` and `pi-runtime.mjs`.

An in-flight run keeps its model; the memory design's fresh-context-per-request
behavior makes the next request safe to switch. Verify restart persistence, a
selection whose account later disappears (shown as unavailable, request fails
visibly), and that no credential value is ever returned over the socket.

## Result

Implemented in `tools/speech/voice.mjs`, `pi-runtime.mjs`, and `accounts.mjs`.

- The `models` socket command returns the image-capable catalog, account state,
  current account/model, and unavailable-account reasons without credential data.
- `select-model` accepts only a currently listed account/model pair and persists it
  in the private Dojo config. Restart restores the selection; an unknown or missing
  account remains selected-but-unavailable rather than rerouting.
- Runtime model configuration changes only before an idle fresh request. Requests
  already running retain their model. The summary model follows the cheapest model
  on the selected account. A no-account runtime starts and reports "Sign in to use
  Dojo" rather than falling back to another provider.

Validation: account-selection, service-socket, runtime, and credential non-disclosure
coverage pass. The full speech suite passes: 178 passed, 1 gated live-keyring check
skipped. The live model was not changed as a test.

## Comments
