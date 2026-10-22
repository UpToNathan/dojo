# Binding panel

Type: task
Status: resolved
Blocked by: 03

A shell surface in the style of the system menu and model picker, shown on
`BindShortcuts` and `ConfigureShortcuts`. It lists the app and each shortcut with its
description and trigger, lets the user press a new key to rebind, clear one, accept or
cancel, and reports conflicts inline. Escape cancels. Cancelling returns response 1;
accepting persists the assignments and emits `ShortcutsChanged`.

While capturing a key, existing grabs must not swallow it (see the spec's open
question). Use the shared palette. Escape and a lost portal request both dismiss it.

## Answer

Pure panel model in `src/shortcuts/panel.rs` (tested) and the GPUI surface in `src/shortcuts_ui.rs`, started from `shell.rs`. Grabs are lifted while the panel is open. The surface itself is not yet seen running in a live session.
