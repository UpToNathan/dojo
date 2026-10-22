# Global shortcuts

Apps on Wayland cannot see keys pressed while another window has focus. The
`org.freedesktop.portal.GlobalShortcuts` portal is the sanctioned route: an app
asks for a shortcut and the desktop tells it when the key goes down and up.
Niri has no implementation, so Dojo supplies one. Design reasons are in
[ADR 0004](adr/0004-niri-grabs-global-shortcuts-over-ipc.md).

```text
app ── xdg-desktop-portal ── Dojo shell (org.freedesktop.impl.portal.desktop.dojo)
                                  │  Request::GlobalShortcuts / ShortcutActivated
                                  └── forked Niri (grabs the keys, consumes press and release)
```

## For the user

When an app first asks for shortcuts, the shell shows a panel at the bottom
right with the app and each shortcut next to the key the app suggested. The panel
opens on Done, so Enter accepts the suggestion. Up and Down choose a row, Enter
on a row waits for the new combination, Backspace clears it and Escape cancels.
A combination another row already uses is refused with a note. While the panel
is open Dojo gives every grabbed key back to the focused app, so a key you have
already bound can be chosen again.

Choices are remembered per app in `~/.local/state/dojo/global-shortcuts.toml`
and not asked about again. An app's own settings button (`ConfigureShortcuts`)
opens the same panel. A key Niri refuses, because `config.kdl` binds it or another
app holds it, shows as not set and is retried the next time the set changes.

An app must register an app id with the portal before using it. Apps outside
Flatpak do this through `org.freedesktop.host.portal.Registry`, and the id must
match one of their `.desktop` files. An app that has not will be refused by
xdg-desktop-portal with "An app id is required"; that is not something Dojo can
change.

## Requirements

- The forked Niri (`scripts/install-niri-fork.sh`). Stock Niri has no
  `GlobalShortcuts` request. Install takes effect at the next login.
- The shell running. It owns the backend's bus name, and nothing starts it on
  demand. With no shell, apps see no GlobalShortcuts portal.
- `yazi-home-manager.nix` routes `org.freedesktop.impl.portal.GlobalShortcuts=dojo`
  and installs `dojo.portal`. `scripts/install-yazi-integration.sh` links both
  from an artifact built with `scripts/check-yazi-module.nix`, then restarts
  `xdg-desktop-portal.service`.

## Checking it

```sh
devenv shell -- cargo test --lib shortcuts
DOJO_XDG_DESKTOP_PORTAL=$(nix-store -qR ~/.nix-profile | grep 'xdg-desktop-portal-[0-9]' | head -1)/libexec/xdg-desktop-portal \
  devenv shell -- cargo test --lib real_frontend -- --ignored
cargo test --manifest-path vendor/niri/Cargo.toml --lib global_shortcut
```

The ignored test runs the real xdg-desktop-portal frontend on a private bus and
walks an app through register, `CreateSession`, `BindShortcuts` and `Activated`.
