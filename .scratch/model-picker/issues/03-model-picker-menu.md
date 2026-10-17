# Add the model picker menu and top-bar agent glyph

Status: resolved
Blocked by: 02

Add the native menu in the Rust shell: a generic agent glyph in the top bar that
opens a layer-shell panel listing the models from issue 02 grouped by account,
with the current selection marked, the carrying account shown per model, and
unavailable accounts explained in a secondary line. Follow `src/system_menu.rs`
and the other panels for theming, keyboard handling, focus, per-display lifecycle
and a `--models` control command.

Talk to the voice service only through its socket, off the GPUI render thread,
and keep catalog parsing and selection logic in a GPUI-independent module with
unit tests. The trigger's render bounds define its input region. Show a failure
from `select-model` in the menu rather than dismissing it.

Verify parsing and selection logic, input geometry, and the panel in an isolated
compositor; do not switch the live session's model as a test.

## Result

Implemented the native panel in `src/model_picker.rs`, wired the agent glyph in
`src/frame.rs`, and added the `--models` control command in `src/main.rs` and
`src/control.rs`.

- The panel loads and selects through the voice Unix socket on a worker thread.
- Models are grouped by account, the selected pair is marked, unavailable
  accounts retain their reasons, and the no-account state explains setup.
- The panel uses the shell theme, keyboard focus/navigation, outside-click
  dismissal, and reports selection failures without dismissing.
- Catalog parsing and account/model selection helpers are GPUI-independent and
  unit-tested.

Validation: `cargo test --bin dojo` passes (126 passed, 2 ignored), including the
three model-picker tests; Rust formatting checks pass. An isolated-compositor/live
panel interaction test was not run in this session.

## Comments
