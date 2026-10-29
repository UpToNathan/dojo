# Notices

Dojo reports what happens on the machine through the notification service (Hokum), as **notices** ([glossary](../../GLOSSARY.md), [ADR-0012](../../docs/adr/0012-notices-go-to-the-notification-service.md)). Today Dojo sends none: there is no low-battery warning, no sign that Wi-Fi or a Bluetooth device connected, and the volume and brightness keys give no feedback (there are no volume keys bound at all).

## Principles

- **Edges, not levels.** A source reports a change. Nothing is said at startup about what was already true: Wi-Fi that is up when the shell starts is not "connected".
- **One owner per event.** The thing that observes a change sends the notice. No source asks another to.
- **Never in the way.** Sending is one best-effort `Notify` call. A missing service, a slow service or a failed call is ignored, and never delays or fails the operation that caused it.
- **Replace, do not stack.** Notices of one kind replace each other (`x-canonical-private-synchronous`). Progress notices carry a `value`.
- **Urgency means something.** Critical is for what needs the person now (critical battery, a failed theme apply they asked for). Normal is a change they would want to know about. Low is a confirmation they caused.
- The sender is `Dojo`; the icon is a theme icon name or none. Hokum draws the card.

## Shared sender

`dojo::notify` (in `src/lib.rs`): a `Notice` (summary, body, urgency, kind, icon, value, timeout) and `send(&Notice)`, a blocking call on the session bus that reuses one connection and falls back to `/run/user/$UID/bus` when `DBUS_SESSION_BUS_ADDRESS` is unset (an ssh session). `dojo-notify`: a command for callers that are not Rust, with `send`, `volume` and `brightness`.

## Events

**Battery and power** (the `dojo` shell, from the host-state loop in `src/frame.rs`):
- Discharging at or below 20%: Normal, "Battery low". At or below 10%: Critical, "Battery critically low", with the time-to-act line. Each fires once per fall; they re-arm when the battery is above 25% or charging.
- Charger plugged in or unplugged: Low, "Charging" / "On battery".
- Fully charged after charging: Low, "Fully charged".
- Never on the first reading.

**Wi-Fi** (`src/wifi_ui.rs`): connected to a network ("Connected to NAME"), disconnected from one ("Disconnected from NAME"). Ethernet and other kinds say their label. Never on the first snapshot. A move between two networks is one notice.

**Bluetooth** (`src/bluetooth_ui.rs`): a device connected or disconnected by name; a connected device's battery falling to 15% or below (once per fall; re-arms above 20%).

**Volume and brightness keys:** `XF86AudioRaiseVolume`, `XF86AudioLowerVolume`, `XF86AudioMute`, and the existing brightness keys (screen and keyboard) call `dojo-notify volume ...` / `dojo-notify brightness ...`, which change the level and send a Low notice of kind `volume` / `brightness` / `keyboard-brightness` with `value` set. Changes made in the bar's audio wave, or by other programs, send nothing.

**Desktop actions:**
- Theme apply (`src/theme/apply.rs`): one notice at the end of an apply the person started (picker or `dojo-theme`), not for the automatic appearance refresh. All targets fine: Low, "Theme applied". Any target blocked or reload failed: Normal, "Theme applied with problems", naming the targets.
- Software updates (`scripts/dojo-update`): the run finished (Low), or finished with failed or blocked steps (Normal, naming them).
- Screenshots: the screenshot bind sends Low, "Screenshot copied" when `utsushot` succeeds.

**Mesh and agent:**
- A node key or roster entry close to expiring (`src/mesh_renew.rs`, replacing its `eprintln!`; once per day per entry as today): Normal.
- Remote input starting and stopping on a peer, and on the operator's side (`crates/dojo-mesh/src/remote_cli.rs`): Low. These run over ssh, where `dojo-notify` finds the session bus by path.
- The agent paused because the person took the keyboard (`src/bin/dojo-agent.rs`, the `InputPaused` sites): Normal, once per pause.

## Not in this pass

- A history or notification centre (Hokum owns that).
- Display layout changes: Dojo only listens to Niri's output events while the displays panel is open.
- "An update is available": nothing checks on a schedule today.
- Peers joining or leaving the mesh: the roster is read only while the mesh panel is open.
- Notifications about notifications (Hokum's own update card).

## Tickets

`issues/01` to `issues/10` are the pass above. The deferred items are `issues/11` to `issues/13`.
