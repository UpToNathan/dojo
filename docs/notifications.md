# Notifications

[Hokum](https://hokum.app) is the session's notification daemon. It owns
`org.freedesktop.Notifications` on the session bus and draws its cards on a layer-shell overlay,
so it works under Niri. Dojo does not implement the interface itself (its StatusNotifier host in
`src/tray.rs` is the tray, not notifications), and no other daemon is installed.

`hokum-home-manager.nix` installs it:

- `nix/hokum.nix` builds it from `vendor/hokum` and the Rune it draws with (`vendor/rune`), both
  Git subtrees pinned to upstream commits (Hokum `22635b2`, Rune `e5a6431`, the one Hokum's own
  lockfile used). Hokum's `Cargo.toml` points at the vendored Rune by path. To take an upstream
  change, `git subtree pull --prefix=vendor/hokum https://github.com/anomalyco/hokum.git REF --squash`
  (and the same for Rune), then rebuild.
- `systemd.user.services.hokum` and `~/.local/share/dbus-1/services/app.hokum.Notifications.service`
  are upstream's `packaging/` files pointed at the store path. The unit is wanted by
  `graphical-session.target`, and the activation file starts it on the first notification if it
  is not running yet. The unit conflicts with `mako`, `dunst` and `swaync`.
- `~/.config/hokum/config.kdl` is not managed. Hokum's settings window creates and edits it in
  place, so Home Manager does not link a read-only copy; `dojo-theme` sets its `theme` line.
- The executable opens Wayland, X11, xkbcommon and Vulkan libraries at runtime (winit and wgpu),
  so `nix/hokum.nix` lists them as `runtimeDependencies`.

## Dojo's changes to the subtrees

- **Rune, glass:** the layer-shell overlay turns the rounded rect of every
  `El::system_material` element into a blur region for `ext-background-effect-v1` (one-pixel
  bands, so the corners stay round, as Dojo's GPUI windows do). Niri blurs what is behind the
  region; the card's own translucent fill is the tint. `layer_shell.rs`, `overlay.rs`.
- **Rune, fonts:** `Family::Named` falls back to the installed fontconfig family of that name
  (`fc-list`), loaded on first use, because Rune reads no system fonts of its own.
  `text.rs`.
- **Rune, tests:** `set_native_help` stays available to test builds so the library's tests
  compile on Linux.
- **Hokum:** a `glass #true` token puts `system_material` on the card, and a build in
  `/nix/store` is treated like a cargo build and never offers updates. `src/theme.rs`,
  `src/card.rs`, `src/update.rs`.
- **Hokum, applications:** the settings window's Applications list is prefilled from the
  installed desktop entries (by `Name=`, with their `Icon=`), after the apps that have sent
  something. A notification is resolved to its entry by its `desktop-entry` hint or its app name,
  and rules match that proper name as well as the name the app sent, so `rule app="Signal"` hides
  `signal-desktop` too and rules written for the sent name keep working. `src/apps.rs`,
  `src/theme.rs`, `src/template.rs`, `src/settings.rs`.
- **Niri:** a `layer-rule` for the `hokum` namespace turns xray off so the blur shows the windows
  under a card and not only the wallpaper (`home-manager.nix`).

The look comes from the theme system: `dojo-theme` has a `hokum` target that publishes the Dojo
theme and selects it in `config.kdl` (see `docs/theme-application.md`). Until the first apply
with that target, Hokum draws its own default look (Capsule).

## Using it

```sh
hokum                 # settings window
hokum check           # problems in config.kdl
hokum dismiss [--all] # close the newest card, or every card
hokum invoke          # do what clicking the newest card does
hokum dnd toggle      # Do not disturb
notify-send test hello   # needs libnotify; any freedesktop client works
```
