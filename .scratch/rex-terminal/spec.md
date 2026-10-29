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
