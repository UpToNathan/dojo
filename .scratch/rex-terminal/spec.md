# Rex terminal

Dojo's terminals become clients of a Rex session instead of windows that own their shell. A terminal you close keeps running in the personal Rex server (`services.rex`, installed by `rex-home-manager.nix`), and opening it again shows the same programs, scrollback and cursor. See [ADR-0011](../../docs/adr/0011-terminals-are-rex-clients.md).

## Pieces

- **`crates/dojo-rex`**: the Rex client library, with no UI. It speaks the documented protocol (the server serves it at `/llms.txt` on its socket): the control connection (`/api/control`, `rex.control.v1`, JSON over WebSocket on the server's Unix socket), data connections (`/api/data`, `rex.data.terminal.v1`: HELLO, WELCOME, REJECT, RESUME), and the terminal stream (a Ghostty snapshot, then VT bytes with `ESC _ rex1;<nonce>;<json> ESC \` messages). It wraps libghostty-vt (linked statically, bindings generated with bindgen) built from the Ghostty revision the installed Rex uses (the `ghostty` input in `devenv.yaml`), because the snapshot format is not yet stable across revisions, to restore the snapshot, apply the stream, read the screen as rows of styled cells, and encode keys with the terminal's current modes.
- **`crates/dojo-terminal`**: the front end, a Rune app. It uses the Rune subtree in `vendor/rune` (shared with Hokum), with two layer-shell fixes marked `Dojo:` in `crates/rune/src/layer_shell.rs`: the keymap is read from offset 0, and a panel that asks for keys takes them exclusively. One binary, two modes:
  - `--panel`: a Wayland layer-shell surface on the overlay layer at a given output, margins and size. Dojo's quick terminal starts it in place of `kitten quick-access-terminal`, exactly as it places kitty today.
  - default: a normal window, the fullscreen terminal app.
  Either mode attaches to a session by label (`--session`), creating it if missing.
- **Shell**: `src/quick_terminal.rs` launches `dojo-terminal --panel` when the `[quick-terminal] client = "rex"` setting says so; kitty stays the default until the Rex client is trusted. Hiding the panel ends the client process, not the session.

## Behaviour

- Opening attaches to session `dojo-quick` (panel) or the label given. A new session runs `scripts/dojo-terminal-session` with no shell wrapper, through `env` carrying the desktop variables the client itself has (`WAYLAND_DISPLAY`, `NIRI_SOCKET`, `DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `SSH_AUTH_SOCK`, `XDG_CURRENT_DESKTOP`, `XDG_SESSION_TYPE`), because the Rex server runs as a systemd user service that may have started before the session. The script's exit trap still tells Dojo the session ended.
- The client claims the block (`session.own_block`) and sets its size from its own surface, so the most recently opened view decides the size.
- Typing goes through libghostty's key encoder into `com.superlogical.terminal.write`, so application cursor keys, the kitty keyboard protocol and bracketed paste follow the terminal's modes.
- A dropped data connection resumes with RESUME and the exact byte count; a refused resume starts over with a new ticket.
- `child_exited` ends the client.
- Nothing runs while the panel is hidden: no client process, no polling. The Rex server is the only thing holding the session.

## Out of scope for the first cut

Mouse reporting, selection and copy, scrollback scrolling, Kitty graphics, the Dojo theme (Rex's default colours first), splits and windows inside a session, remote Rex servers over Tailscale.

## Sessions, tabs and splits

`dojo-terminal` shows a whole Rex session the way Rex for Mac does: the session's Rex windows are tabs, a window's layout of blocks is split panes, and a session picker switches the window to another session.

- **Super+T** runs `dojo-terminal --reattach`: it opens the most recently used session (Dojo keeps the order in `$XDG_STATE_HOME/dojo/terminal/recent`), or focuses the window already showing it. With no sessions it creates one. **Super+Shift+T** runs `dojo-terminal --picker`, which opens on the picker.
- **Session picker** (Ctrl+Shift+S, Rex's "Change Session…"): every session, most recent first, with its label, directory, running program, and a marker when a program in it reported `done`, `blocked` or `error` through the Program Status Protocol. Typing filters; Enter switches the window to the selected session; a typed name that matches nothing creates a session with that name; Escape closes the picker.
- **Tabs**: a tab bar shows the session's windows when there is more than one. Ctrl+Shift+T new tab, Ctrl+Shift+W close tab, Ctrl+Tab / Ctrl+Shift+Tab next / previous, Ctrl+1…9 go to a tab.
- **Splits**: the active window's layout is drawn as panes, each its own terminal stream sized to its rectangle; the focused pane gets the keys and a visible border. Ctrl+Shift+D split right, Ctrl+Shift+E split down, Ctrl+Shift+arrows move focus, Ctrl+Shift+Z zoom, Ctrl+Shift+X close the pane. Clicking a pane focuses it.
- Tabs, splits and focus are Rex's own (`session.new_window`, `session.new_split`, `session.focus_*`, `session.zoom_block`, `block.close`), so every client of the session sees the same arrangement.
- The window title is the session's label, and the Wayland app id is `dojo-terminal`.
- The quick terminal stays on session `dojo-quick` and uses the same client, tabs and splits included.
