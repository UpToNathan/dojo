# Add the onboarding and account sign-in panel

Status: resolved
Blocked by: 03, 04

Add the shell side of sign-in in [the spec](../spec.md). Show account rows with
state and Sign in or Sign out in the model picker menu, and an onboarding view when
no account is usable, including a needs-setup state on the top-bar glyph.

Show the ChatGPT device code and verification address with a cancel action, and
fall back to a paste field when the service asks for one. Take the OpenCode key in a
masked native editor modelled on the Wi-Fi password editor, clear it on submit or
dismissal, and show a validation failure without dismissing. The shell never reads
or stores credentials.

Keep state and flow logic in a GPUI-independent module with unit tests, and verify
the panel in an isolated compositor with a fake service. Do not sign out of the
live session as a test.

## Resolution

- The menu is a compact two-step picker in the style of the Power and Wi-Fi panels
  (shell extension, content-sized, keyboard driven). `src/picker_menu.rs` holds its pages,
  rows and cursor windowing; `src/model_picker.rs` renders it.
- `src/sign_in.rs` holds the GPUI-independent flow: login status to progress mapping,
  masked/plain input buffers, ChatGPT and OpenCode-key flows, needs-setup and account
  action rules (13 unit tests). `src/model_picker.rs` renders account rows with Sign in /
  Sign out, the onboarding intro, the device-code card (copy, cancel, browser
  alternative), the paste field, and the masked key editor, which stays open with the
  reason on rejection and is cleared on submit, cancel and dismissal.
- The voice service gained a read-only `login-status` action (public progress only).
- The top-bar glyph shows a dot while no account is usable. The shell reads account
  state at startup and every five minutes, and whenever the picker loads a catalog.
- Verified in an isolated nested compositor against `tools/agent-input/fake-voice.mjs`
  with `run-nested.sh --model-picker` (real typing through wtype and clicks through
  wlrctl, so it needs `DOJO_NESTED_AGENT_INPUT=0`): needs-setup glyph, onboarding, key
  editor, masked entry, rejected key, device code, cancel, completion, sign-out and
  selection fallback. The live service and a release shell were also checked read-only;
  no live account was signed out.
- The live keyring check found two bugs, both fixed: the real `secret-tool` exits 1 with
  no output for a missing item (treated as an error, so every signed-out account read as
  unavailable), and three account tests used the real keyring and left fixture
  credentials in it.

## Comments
