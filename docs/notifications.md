# Notifications

[Hokum](https://hokum.app) is the session's notification daemon. It owns
`org.freedesktop.Notifications` on the session bus and draws its cards on a layer-shell overlay,
so it works under Niri. Dojo does not implement the interface itself (its StatusNotifier host in
`src/tray.rs` is the tray, not notifications), and no other daemon is installed.

`hokum-home-manager.nix` installs it:

- `nix/hokum.nix` packages the pinned upstream release executable (the same approach as
  `nix/rift.nix`). Bump the version, URL and hash together.
- `systemd.user.services.hokum` and `~/.local/share/dbus-1/services/app.hokum.Notifications.service`
  are upstream's `packaging/` files pointed at the store path. The unit is wanted by
  `graphical-session.target`, and the activation file starts it on the first notification if it
  is not running yet. The unit conflicts with `mako`, `dunst` and `swaync`.
- `~/.config/hokum/config.kdl` is not managed. Hokum's settings window creates and edits it in
  place, so Home Manager does not link a read-only copy. v0.1.1 has no update checker; once a
  release that has `check-for-updates` is pinned, set it to `#false` in `behaviour`, because the
  executable lives in the Nix store and a new build comes from bumping `nix/hokum.nix`.
- The executable opens Wayland, X11, xkbcommon and Vulkan libraries at runtime (winit and wgpu),
  so `nix/hokum.nix` lists them as `runtimeDependencies`.

Its look comes from the theme system: `dojo-theme` has a `hokum` target that publishes the Dojo
theme and selects it in `config.kdl` (see `docs/theme-application.md`). Until the first apply with
that target, Hokum draws its own default look (Capsule).

## Using it

```sh
hokum                 # settings window
hokum check           # problems in config.kdl
hokum dismiss [--all] # close the newest card, or every card
hokum invoke          # do what clicking the newest card does
hokum dnd toggle      # Do not disturb
notify-send test hello   # needs libnotify; any freedesktop client works
```
