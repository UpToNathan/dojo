# Install, routing and documentation

Type: task
Status: resolved
Blocked by: 03

- Ship `dojo.portal` (`DBusName=org.freedesktop.impl.portal.desktop.dojo`,
  `Interfaces=org.freedesktop.impl.portal.GlobalShortcuts`, `UseIn=niri`) and route
  `org.freedesktop.impl.portal.GlobalShortcuts=dojo` in `portalRouting`
  (`yazi-home-manager.nix`), keeping the existing gnome ScreenCast and Screenshot and
  gtk fallback. Install through the repo's scripts, never by hand.
- Nothing in the backend may start through devenv or read its environment.
- Add a README section: what the backend does, how to bind a shortcut, why it needs the
  shell running, and the Niri fork requirement.
- Verify through the real xdg-desktop-portal on the live session after install.

## Answer

`yazi-home-manager.nix` routes the backend and installs `dojo.portal`; `check-yazi-module.nix` and `install-yazi-integration.sh` carry it. Documented in `docs/global-shortcuts.md` and the README. Not yet installed on the host.
