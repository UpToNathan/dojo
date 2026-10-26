# The send panel covers the 1Password approval it is waiting for

Type: task
Status: resolved

Every send signs with the node key in 1Password's agent, and 1Password asks to approve it in an ordinary window. The send panel (`src/send_window_ui.rs`) is a full-screen `Overlay` layer-shell surface with `Exclusive` keyboard interactivity, so that window opens underneath it and cannot be reached. The send waits, ssh gives up (exit 255, `[exit 255]` in the audit log, 3 to 26 seconds later), and the panel sits on "Sending…". Seen on 2026-10-26 sending a kitty window three times and a T3 Code window once. Once the approval was given by other means, the same calls took about a second.

Fix so that a call waiting for a person never hides what the person has to act on:

- While a send or `remote` call is outstanding, the panel must not cover other windows or hold the keyboard. Hide it (or drop it below ordinary windows and release the keyboard) and bring it back with the result, or show a small status that takes no input.
- Say what is being waited for ("approve the node key in 1Password") and report an unapproved key or a locked agent as exit 3 (a person has to act), not as exit 255 or a timeout, as `docs/mesh.md` already says for the command line.
- Optionally ask for the approval before the panel appears, so the dialog is never competing with it.

1Password's own setting for how long an approval lasts is the operator's choice and not something to change from here.

## Answer

Hiding the panel while a send waits (`window.hide()`) was tried and rejected: it flickers and is not a real fix. The panel now stays but lends its input: while a send runs it sets an empty input region and drops its keyboard interactivity to `None` (a runtime setter added to the vendored GPUI), so the 1Password dialog under it takes clicks and keys, and takes both back with the result (`Panel::lend_input` in `src/send_window_ui.rs`).

Still open: saying what is being waited for, and reporting an unapproved key as exit 3 instead of ssh's 255, which the exit code alone cannot tell apart.
