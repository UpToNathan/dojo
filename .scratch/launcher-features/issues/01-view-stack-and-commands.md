# Launcher view stack, commands and generalized history

Status: resolved

Add a view stack inside the launcher, a registry of Commands, a Suggestions group in the empty launcher, and a generalized history store (today `src/launch_history.rs` keys applications only; commands, emoji, clipboard entries and bangs need the same decay and learned-term model under their own namespaces). Esc and Backspace-on-empty pop a view. `dojo --launcher <view>` opens straight into a view. Commands can be hidden. Root results become a non-app result type so inline rows can follow.

## Comments

Built: rows (apps, commands, inline rows) replace bare indices; view stack with Escape/Backspace; Suggestions group in the empty launcher ordered by use; Ctrl+H hides a suggestion; `dojo --launcher-view <name>`; the history store keys commands and view items. Tests: root ranking (12), launcher stack with a fake view, control protocol, hiding.
