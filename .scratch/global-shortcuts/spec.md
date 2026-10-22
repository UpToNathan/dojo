# Global shortcuts portal backend

Dojo's shell implements `org.freedesktop.impl.portal.GlobalShortcuts`, so a Wayland
app can ask for a system-wide hotkey and be told when it is pressed and released,
on Niri, without Niri shipping the portal. Prompted by
https://x.com/thdxr/status/2113072161531572675: Niri has no GlobalShortcuts, so
apps fall back to `/dev/input` (watch-only, the focused app still gets the key) or
to nothing.

## What exists today

- Niri is vendored and patched (`vendor/niri`). Speech already uses a Dojo patch,
  `spawn-on-release`, which tracks the release by physical key code. Binds are
  config-only: there is no way to add one at runtime.
- Niri IPC has requests, an event stream, and per-connection state that is torn down
  when the socket closes (`ClientCtx::drop` destroys the agent seat).
- The shell is resident, already uses zbus, and has portal client code
  (`src/theme_picker/portal.rs`) and private-bus test fixtures
  (`src/bluetooth/tests.rs`, `src/wifi.rs`).
- Portal routing is `niri-portals.conf`, rendered from `yazi-home-manager.nix`
  (`portalRouting`). xdg-desktop-portal is 1.22.1 and the gnome, gtk, darkman and
  termfilechooser backends are installed through Home Manager. No backend offers
  GlobalShortcuts for Niri.

## Decision: the compositor grabs the keys

Three ways to learn that a key was pressed:

1. **Runtime grabs over Niri IPC (chosen).** A client sends the compositor a set of
   shortcut grabs; the compositor matches them after config binds, consumes both the
   press and the release, and reports `ShortcutActivated` / `ShortcutDeactivated` on
   that connection's event stream. Grabs die with the socket. This gives the portal
   its real semantics (Activated and Deactivated, key not leaked to the focused app)
   and handles conflicts and cleanup with no file edits.
2. **Generate KDL binds** that `spawn` / `spawn-on-release` a `dojo` CLI call into the
   control socket. Needs no compositor change and is how voice works today, but it
   rewrites a user-owned config and reloads it, cannot report a conflict at bind time,
   costs a process spawn per edge, and leaves stale binds if the shell dies. Kept as
   the fallback if the IPC patch proves too costly to carry.
3. **Read `/dev/input`.** The thing the tweet complains about. Rejected.

Cost of 1: another patch carried in the Niri fork on every upstream sync. The agent
seat and `spawn-on-release` are precedent. Worth an ADR when issue 02 lands.

## Behaviour

**Portal side** (the shell owns `org.freedesktop.impl.portal.desktop.dojo`,
object `/org/freedesktop/portal/desktop`, interface version 2):

- `CreateSession` registers a session keyed by `session_handle`, remembering `app_id`.
  Sessions end on `Close` or when the caller leaves the bus.
- `BindShortcuts` receives `(id, {description, preferred_trigger})`. The trigger is a
  hint in the XDG shortcuts format (`CTRL+ALT+a`, `LOGO+space`). The user decides, in
  a Dojo panel (issue 04). Response code 0 with the granted shortcuts, 1 if cancelled.
- `ListShortcuts` returns what is bound for that session with the trigger shown to the
  user (`trigger_description`).
- `ConfigureShortcuts` (v2) takes only the session, the parent window and options, and has no
  reply: it opens the same panel for an existing session, and a change arrives as
  `ShortcutsChanged`.
- `Activated` / `Deactivated` carry the session, shortcut id and a millisecond timestamp
  (undefined base), and follow the key's press and release. Key repeat never produces extra `Activated`.
- `ShortcutsChanged` fires when the user rebinds from the panel.
- Assignments persist per `app_id` in Dojo's state directory, so a rebinding survives
  restarts. The same shortcut id from the same app gets the same trigger again without
  a prompt.

**Compositor side:**

- A grab is a key in Niri's syntax plus an opaque id. A connection replaces its whole
  set in one request; the reply names any key that could not be granted (already a
  config bind, a hardcoded bind, or another connection's grab). Config binds win.
- Grabs are inert while the session is locked and while a shortcuts inhibitor is
  active (same as an ordinary bind), and never intercept the hardcoded VT and power keys.
- Press and release are paired by physical key code, so releasing modifiers first does
  not strand a shortcut in the active state.

## Out of scope

Mouse, gamepad and modifier-only triggers. The permission store and sandbox prompts.
Migrating the existing speech binds to the portal (they stay in `config.kdl`).
`InputCapture` and `RemoteDesktop`. Other compositors.

## Resolved questions and remaining risks

- **`app_id` for unsandboxed apps.** Resolved by running the real frontend: xdg-desktop-portal
  1.22.1 refuses a host app with "An app id is required" until it registers one through
  `org.freedesktop.host.portal.Registry.Register`, and the id must match a `.desktop` file. The
  backend therefore always receives a real id and persistence is keyed on it. An empty id is
  still tolerated, in one shared bucket.
- **Dialog without focus.** Resolved: while an interactive question is open the backend replaces
  its grabs with an empty set, and restores them when the question ends.
- **Timestamps.** Resolved: milliseconds, undefined base.
- **Shell not running.** Accepted: the backend name is owned by the resident shell. If the
  shell is down the frontend returns an error and the app sees no portal. No D-Bus activation
  through devenv (see AGENTS.md, runtime never uses devenv).
- **Niri restart.** If Niri's socket closes the backend logs it and stays up; shortcuts do not
  resume until the shell restarts, which a Niri restart (a new session) does anyway.
- **Broadcast events.** Every IPC event stream sees every `ShortcutActivated`; see ADR 0004.

## Verification

Pure logic tests (trigger parsing both ways, conflict resolution, persistence). Niri
unit tests for matching, release pairing, lock, inhibitor and disconnect. A private-bus
zbus test where a fake frontend drives the backend. A nested Niri run
(`tools/agent-input/run-nested.sh` pattern) where a small portal client binds a key,
`wtype` presses it, the client sees Activated then Deactivated, and a second focused
client receives no key event. Finally the real frontend: the same client through
xdg-desktop-portal 1.22.1 on the live session after install.
