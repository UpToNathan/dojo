# Launcher features

The launcher grows from an application search into a Raycast-style command surface, while staying one layer surface and not becoming the front door for Dojo's own panels (Wi-Fi, Bluetooth, Displays, Theme, Models, Shortcuts and the rest keep their own keybinds and are not launcher results).

Ideas were compared against oma-cast (typecraft-dev/oma-cast, no licence file, so ideas only and no code is copied). Frecency and learned search terms already shipped (`src/launch_history.rs`).

## Vocabulary

- **Command**: a built-in launcher entry that opens a view (Emoji, Clipboard History, Windows, Snippets, Pick Color, Processes, Services). Commands appear as Suggestions in the empty launcher and rank like any result when typed.
- **View**: a screen pushed inside the launcher (search box, list, action on Enter). Esc, or Backspace on an empty search, pops it; Esc at the root closes the launcher. No view opens a new window.
- **Inline row**: a result produced by an explicit trigger in root search (calculator, bang, `?` agent prompt, colour, dev utility). It never appears for ordinary text.

## Decisions so far

- Everything is a launcher feature. There are no extra windows; views reuse the launcher surface and its hide/show reuse.
- Commands are also shown in the empty launcher as a "Suggestions" group above frecency-ordered apps. Order comes from the history store; the fixed starting order is Emoji, Clipboard History, Windows, Snippets. A command can be hidden.
- Direct keybinds may open the launcher straight into a view (`dojo --launcher <view>`).
- The same history model ranks commands and view items (emoji recents, clipboard re-copies, bangs).
- Enter copies to the clipboard and closes. Automatic paste (injecting Ctrl+V) is a later opt-in, not part of the first versions.
- The `?` agent prompt is plain text sent to the agent runtime; the shell's speech bubbles show the reply (there is no separate Agent panel). It must not need a microphone or the voice path. The runtime is split out of `dojo-voice` first (issue 02), so it runs without a mic and voice becomes one input to it.
- Bangs resolve inside the launcher and open the final URL in Helium (`helium` wrapper, `xdg-open` fallback). Helium's native bangs are omnibox-only and cannot be driven from outside, so the launcher carries its own tracked list (draft in `bangs.md`) plus a user file at `~/.config/dojo/bangs.toml` that wins on a clash. Larger lists need a licence check before use.
- Colour picking uses Niri's native `PickColor` request, no extra tool. Conversions reuse the oklch code in `src/theme/refined.rs` where possible.
- Process management is inspired by pitchfork (project daemons, ports, ready checks) and process-compose, but pitchfork is not adopted. Processes (list and kill) and Services (systemd user units) are the launcher's scope.
- Dev utilities are wanted, as inline rows.
- Media controls may live in the shell's left sidebar, not the launcher. Undecided.

## Order

01 foundation, 02 runtime split, 03 emoji, 04 clipboard history, 05 window switcher, 06 snippets, 07 calculator, 08 bangs, 09 agent prompt, then 10 colour, 11 processes and services, 12 dev utilities.

## Not decided

Quick timer, launcher polish (aliases, pins, hiding apps; hiding commands is part of 01), media controls, a dev-services adapter for pitchfork or process-compose.

## Out of scope

File search, translator, dictionary, AI chat inside the launcher (the Agent panel exists), window management (Niri), expanding snippets as you type in other apps (no safe Wayland mechanism).
