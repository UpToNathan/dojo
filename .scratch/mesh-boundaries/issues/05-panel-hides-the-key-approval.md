# The send panel covers the 1Password approval it is waiting for

Type: task
Status: claimed

Every send signs with the node key in 1Password's agent, and 1Password asks to approve it in an ordinary window. The send panel (`src/send_window_ui.rs`) is a full-screen `Overlay` layer-shell surface with `Exclusive` keyboard interactivity, so that window opens underneath it and cannot be reached. The send waits, ssh gives up (exit 255, `[exit 255]` in the audit log, 3 to 26 seconds later), and the panel sits on "Sending…". Seen on 2026-10-26 sending a kitty window three times and a T3 Code window once. Once the approval was given by other means, the same calls took about a second.

Fix so that a call waiting for a person never hides what the person has to act on:

- While a send or `remote` call is outstanding, the panel must not cover other windows or hold the keyboard. Hide it (or drop it below ordinary windows and release the keyboard) and bring it back with the result, or show a small status that takes no input.
- Say what is being waited for ("approve the node key in 1Password") and report an unapproved key or a locked agent as exit 3 (a person has to act), not as exit 255 or a timeout, as `docs/mesh.md` already says for the command line.
- Optionally ask for the approval before the panel appears, so the dialog is never competing with it.

1Password's own setting for how long an approval lasts is the operator's choice and not something to change from here.

## Answer (partial)

`src/send_window_ui.rs`: a send that takes longer than 1.5 seconds hides the panel (`window.hide()`, which unmaps the layer surface and releases the keyboard) and shows it again with the result, so a dialog opened by the wait is reachable. A quick send never hides it. Built and the main crate's tests pass; not run against a real 1Password prompt yet, and not installed.

Still open: telling the operator what is being waited for while the panel is hidden, and reporting an unapproved key as exit 3 instead of ssh's 255. Neither can be told from ssh's exit code alone, so this needs the node key's approval state or a pre-flight signing call.
