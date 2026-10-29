# Dojo

## Desktop configuration

This repository is the source for the desktop's devenv inputs and definition,
Home Manager module, Neovim and Rush configs, wallpaper, and custom PAM, Nix,
1Password, systemd, tmpfiles, and udev configuration. `~/.config/devenv` keeps
symlinks to maintained files here; its `.devenv` directory is local generated
cache/state and is not part of the source configuration.

Home Manager autostarts Dojo through a wrapper that supplies devenv's graphics
and Wayland library paths. Build and install the optimized desktop with `devenv shell -- cargo build --release --bin dojo`,
then `devenv shell -- scripts/install-dojo-release.sh`. The installer keeps the previous
binary and its runtime library roots. Its per-shell allocator setting returns large
freed buffers to the system; an explicitly supplied mmap threshold takes precedence. Run
`devenv shell -- scripts/check-dojo-autostart.sh` after login to check the actual
autostart service, its process and the mapped desktop frame.

Run `devenv shell` or `devenv eval ...` from this directory. The account, home
directory, hostname and checkout path are read from the host when devenv
evaluates (`$USER`, `$HOME`, `/etc/hostname` and the checkout's own location),
so the tracked files name no machine and the checkout can live anywhere. Run
`scripts/install-system-config.sh` to link the maintained system files into
place; it preserves replaced files as timestamped backups. See
`system-config/README.md` for tracked inputs versus generated runtime state.

To bring a new host up from a checkout, run `scripts/bootstrap-host` (`system`, `user`, `finish`,
or `all`). It chains the installers, builds, speech packages, Home Manager deployment and
session setup, and detects the NVIDIA driver (`/etc/dojo/nvidia-driver.json`, written by
`scripts/install-nvidia-gl.sh`; rerun it after the NVIDIA package updates). It expects the Arch
packages, Nix and devenv to be installed already. `scripts/bootstrap-host check`
confirms the running session can activate the file-picker and portal services. The last step of
setting a machine up is joining the Dojo mesh: `all` offers it at a terminal, and
`scripts/bootstrap-host onboard` runs it on its own ([Onboarding a machine](docs/mesh.md#onboarding-a-machine)).

Home Manager also installs T3's bundled CLI, and `scripts/install-t3code.py` installs the app it runs. The desktop app owns its local
backend; Dojo does not autostart a separate T3 server. See
[T3 backend setup](docs/t3-server.md). [Rift workspaces](docs/rift.md) describes the pinned
CLI and OpenCode workspace plugin installed through Home Manager and devenv.

Run `devenv shell -- dojo-update --dry-run` to preview the software update plan.
The default version buffer is three days; Pi, OpenCode, Codex, Claude, and T3 Code
are exempt. The updater also refreshes the tracked package, service, and
privileged-file inventory. See [software updates and restoration](docs/system-update.md)
and [the system inventory](docs/system-inventory.md).

Notifications come from [Hokum](docs/notifications.md), built from the `vendor/hokum` and
`vendor/rune` subtrees and run as a user service that owns `org.freedesktop.Notifications`.

Clipboard history and persistence are separate services: Clipvault stores past
entries, while `wl-clip-persist` retains the regular clipboard in memory after
the source app closes. Both run with the graphical session; primary selection
is unchanged. To verify in a live Wayland session, run
`python3 scripts/check-clipboard-persistence.py` (replaces the current clipboard).

## Project

Local speech capture and transcription are extracted from the pinned Hex
subtree into [dojo-speech](crates/dojo-speech/README.md). Build that independent
package through devenv; its CLI supports model installation, WAV transcription
and microphone capture.

Dojo is a Niri desktop shell implemented with GPUI. Build and test it with
`cargo check` and `cargo test` inside devenv.

### Forked desktop dependencies

`vendor/niri` contains the pinned compositor fork. `vendor/zed` contains the
pinned GPUI source and its workspace dependencies. Dojo imports GPUI from local
paths, so changes to the fork are compiled directly. `vendor/keywork` and
`vendor/keywork-compositor` are reference subtrees for independent agent input.
See [the input analysis](docs/agent-input-analysis.md) and
[the implementation spec](.scratch/agent-input/spec.md). The
[agent input guide](docs/agent-input.md) covers the controller API and nested
demonstration.

`dojo-agent` exposes the independent controller as a local stdio MCP server for
AI clients. Its shared Unix socket service starts on demand and exits after all clients
disconnect. Idle seats and captures expire while connected clients remain
available. The service does not require systemd. Build with
`devenv shell -- cargo build --bin dojo-agent`; see [client setup](docs/agent-input.md#connect-an-ai-client). Each server retains its
own cursor and keyboard seat across tool calls.

Build the compositor inside the same development environment:

```sh
devenv shell -- cargo build --manifest-path vendor/niri/Cargo.toml --no-default-features
```

This development build uses Niri's nested Wayland backend when launched inside
an existing Wayland session. It does not change the installed Niri package.
Agent input is experimental and requires `DOJO_AGENT_INPUT=1` in the compositor
environment. The controller holds a persistent IPC connection; separate
one-shot requests cannot preserve the controller's seat.

To install the full release compositor for the next UWSM Niri login, follow
[the session switch guide](docs/niri-session.md). The installer preserves the
current session and keeps the upstream package available for rollback.

The desktop frame uses the Top layer so fullscreen applications cover it.
The Dojo Niri fork pauses focus follows mouse while primary-seat keys are held
and for 750 milliseconds after keyboard activity. Subsequent mouse motion resumes
window and monitor focus; explicit clicks still change focus immediately.
`devenv shell -- tools/agent-input/run-nested.sh --fullscreen` checks rendered
fullscreen edges and frame restoration in an isolated session.

Battery observation follows UPower events, with a 60-second reconciliation while
subscribed and a 10-second fallback when unavailable. Vigiland detection keeps
its independent 10-second reconciliation deadline; battery events never trigger
process scans. While Vigiland runs, Dojo revalidates the cached process's owner
and name instead of walking all of `/proc`.

Development builds optimize the shader/pipeline dependencies (`naga`,
`wgpu-core`, and `gpui_wgpu`) because unoptimized window initialization can
dominate launcher opening time. Dojo's own code stays in the normal debug profile.

## Theme configuration

Dojo reads `$XDG_CONFIG_HOME/dojo/config.toml` (normally
`~/.config/dojo/config.toml`). The root `config.toml` contains the fallback palette;
copy or symlink it to that location. Dojo never rewrites this file.

The `[theme.colors]` table defines `primary` (accent), `background`, `foreground`,
`background_highlight` (hovered surfaces), `border` (including separators), and
`foreground_muted` (disabled text). Colors use `#RRGGBB` strings. Missing fields
use the current default palette; unknown fields and malformed colors are errors.

Native filesystem events reload edits without restarting. Invalid edits keep
the last working palette and log the error; invalid initial config prevents
startup. Removing the file restores defaults. Runtime state remains separately
stored under `$XDG_STATE_HOME/dojo/state.toml`.

These are shared semantic color roles. Selected runtime themes override this
fallback; application exporters are described in `docs/theme-application.md`.

## Wallpaper palette extraction

`dojo-palette` ports Aether's extraction and all 23 palette modes into Rust,
including normal-mode monochrome detection, light/dark generation, chromatic
weighting, OKLab median-cut clustering, OKLCH palette shaping, and contrast
correction. The upstream algorithm is pinned to
`29d07b1a8f474c77bf02aece948373f40abed105`; attribution is in `licenses/`.

Inside devenv:

```sh
cargo run --bin dojo-palette -- ~/Pictures/astro.jpg --mode normal --dark
cargo run --bin dojo-palette -- ~/Pictures/astro.jpg --mode ocean --light
cargo run --bin dojo-palette -- --list-modes
```

Output is a JSON palette of 16 ANSI colors. The command does not apply a theme,
change the wallpaper, rewrite config, or reload other apps. The shared library
interface is `dojo::extraction::{extract, extract_pixels, Mode, Palette}`.

`cargo test --lib` checks every mode against Go-generated reference palettes in
both light and dark variants, plus quantization and alpha-aware resampling.
`python3 scripts/regenerate-aether-fixtures.py` explicitly downloads the pinned
Go source and regenerates the fixtures; normal Rust tests require no network or Go.
See `docs/theme-extraction.md` for parity limits and the next integration steps.

## Applying themes

`dojo-theme` applies static presets, retains selected wallpaper snapshots,
exports application formats and coordinates reloads without rewriting the user
config. Wallpaper-derived themes use the refined recipe with mode-dependent
colors fitted into semantic hue ranges. Run `dojo-theme --help`; see `docs/theme-application.md` for target readiness, opt-in
Home Manager setup, live-update contracts and rollback.

GNOME Keyring's Secret Service component is started with the graphical session
for desktop applications. They discover `org.freedesktop.secrets`
on the user session bus; no extra secrets-provider CLI argument is required.
Starting the daemon does not bypass keyring locking or unlock prompts.
GCR's SystemPrompter and PrivatePrompter D-Bus activation files are also
provisioned: the daemon alone cannot create or unlock a keyring if its prompt
service is absent. Enter keyring passwords only in the native dialog.

The paintbrush beside the system tray (or `dojo --theme`) opens Wallpaper,
Presets, and Derive tabs. Wallpaper changes can keep the existing palette;
theme Apply shows per-app setup/reload results. Choose image opens the desktop's
FileChooser portal rather than an embedded file browser. The portal follows the
configured system picker (currently GTK); selecting a file only updates the
preview, and cancelling leaves it unchanged. Presets switch darkman
to their authored light/dark mode. Tokyo Night/Catppuccin use upstream native
Neovim plugins; other presets still need authored-role fidelity validation.
See `docs/zen-dbus-exhaustion.md` for the separate wluma reply-accumulation fix.

## Application launch lifetime

Dojo parses desktop-entry command arguments itself; it does not use `gio` or a
shell interpreter. In an active UWSM session it delegates lifecycle/environment
handling to `uwsm app -t service`. Otherwise it uses a separate systemd user scope.
Apps have independent process groups/cgroups and journal logging and survive a
Dojo restart. D-Bus-only entries use explicit activation; timeout is not treated
as permission to spawn a second instance. Use `dojo --quit` for shell shutdown.

## Top bar audio

The seven-stroke wave to the left of the corner icons shows the default output's volume. Hover to expand it and reveal the percentage. Drag right to raise volume or left to lower it; scroll for small changes. Click to mute or restore the previous volume. Adjustments stay between 0% and 100%.

The wave follows external volume, mute and default-output changes through `pactl subscribe`; `wpctl` reads and adjusts the default PipeWire output on a worker. Volume writes share a fixed 25-millisecond batching window with blocking channel waits, not polling; the last drag value is delivered without waiting for mouse-up. Mute clicks preserve command order and flush pending adjustments early. The wave stays still when idle. Bluetooth and output selection are not included.

For a live interaction check, identify the wave's center in logical output coordinates, then run `python3 scripts/check-audio-wave.py --output eDP-1 --x X --evidence /tmp/dojo-audio-check`. The check temporarily adjusts audio, verifies click, drag and scroll against system volume, saves screenshots, and restores the original volume and mute state.

## Bluetooth dock

Click the Bluetooth icon beside the audio wave to open the native device dock. Device rows expose Connect, Disconnect or Pair directly, with connected devices first and battery when available. Select a row to view its address. Nearby results show named devices first; unnamed devices remain available through an explicit reveal. Find a device searches nearby for up to 30 seconds. Closing the dock releases Dojo's discovery session and cancels unfinished pairing. Pairing confirmations, PINs and passkeys use the dock's keyboard-capable panel.

Dojo uses the system BlueZ D-Bus service, with event updates and no automatic trust or pairing approval. Device-state signal bursts share a fixed 100-millisecond refresh window; pairing prompts and action feedback remain immediate. Local prompt edits do not fetch the full device list. On this Arch host, run `bash scripts/setup-bluetooth.sh` to install BlueZ and enable its service; sudo prompts for your password in the terminal. The dock updates when BlueZ starts. Dojo's private D-Bus tests exercise connection errors, discovery ownership, authentication, cancellation, daemon restarts and shutdown without touching real devices.

## Shell controls

The top-center clock shows the local date and time. Click the Wi-Fi icon directly
left of Bluetooth to view networks, scan, connect or disconnect, and control the
radio. Personal networks use a masked password prompt; saved profiles come from
NetworkManager. Passwords stay out of command arguments and logs.

The network icon and menu follow NetworkManager's primary connection. When a
phone's USB tether carries the connection, the icon changes to Ethernet and the
menu shows USB Ethernet, the connection name, and interface. Wi-Fi controls remain
available. Removing the tether restores the Wi-Fi display when Wi-Fi becomes
primary again. NetworkManager signals trigger updates, with a five-second
reconciliation fallback.

For an iPhone, run `bash scripts/setup-usb-tethering.sh` once to install USB pairing
support and load the Ethernet driver. Enable Personal Hotspot, unlock the phone,
and accept Trust if prompted. Reconnect the cable if NetworkManager does not show
the USB adapter. The setup command requires sudo; pairing records remain in the
system's runtime state. Android USB tethering uses the kernel's USB networking
drivers and NetworkManager.

Click the power icon in the bottom-right corner for Suspend, Log out, Restart,
and Shut down. Log out, Restart, and Shut down require confirmation inside the
menu. Escape cancels or closes it. The terminal shortcuts are
`dojo-desktop --wifi` and `dojo-desktop --system-menu`.

Click the agent glyph in the top bar or run `dojo --models` to open the model
picker, a compact two-step menu. The first step lists accounts with the model in use
(or Sign in); choosing a signed-in account shows its models, with Back and Sign out.
Up, Down, Enter and Escape work throughout, and long model lists show a short window
that follows the cursor. On a long list, type to filter by name or id (Escape or Ctrl+U
clears it), and use Page Up/Down, Home and End to jump; the mouse wheel moves one row
per notch, and touchpads one row per row's worth of travel. Choosing a model applies to future agent requests, closes the
menu, and leaves a request already running on its original model. Dojo never reroutes a
failed request to a different account.

ChatGPT signs in with a device code shown in the menu (or the browser, with a pasted
redirect when port 1455 is busy). OpenCode Go and Zen take an API key typed into a
masked field; the voice service checks it and stores it in the system password store
(GNOME Keyring), never in the shell. With no usable account the glyph shows a dot. To
check the menu against a fake service in an isolated compositor, run
`DOJO_NESTED_AGENT_INPUT=0 nix shell nixpkgs#wtype nixpkgs#wlrctl -c devenv shell -- tools/agent-input/run-nested.sh --model-picker`.

## Monitor settings

Click the monitor icon in the top bar or run `dojo-desktop --displays` to arrange
monitors inside the shell. Drag the monitor boxes, choose resolution and refresh
rate, and adjust scaling. Apply previews the changes for 15 seconds; Keep saves
them. See [monitor settings](docs/displays.md) for rollback and persistence.

## Application launcher

Run `dojo --launcher` to toggle the native GPUI launcher. In a development
checkout use `./target/debug/dojo --launcher` from this directory after building.
The command contacts the resident shell; if none is running, it starts one.
No compositor keybindings are changed automatically.

Type to fuzzy-search applications, use Up/Down to select, Enter to launch, and
Escape or an outside click to dismiss. Search supports paste, selection,
grapheme-aware deletion, and Wayland text input. The launcher uses the shared
theme palette. Desktop-entry icons are resolved in the background using the
installed icon theme, with absolute icon paths and hicolor fallbacks supported.
Missing icons use a generic placeholder; icon loading does not block matching.

Desktop entries are discovered from XDG application directories in precedence
order, including localized names, keywords, visibility and `TryExec` filtering.
Discovery and Nucleo matching run on a worker with a single latest-query slot;
stale results cannot launch an application. Opening the launcher refreshes the
catalogue so newly installed desktop entries appear without a Dojo restart.
Applications you launch and the searches that found them rank first. The launcher
also has commands (emoji, clipboard history, windows, snippets, colour picking,
processes, services), bangs, a calculator and a `?` prompt to the agent; see
[the launcher guide](docs/launcher.md). Desktop actions, PATH-only commands and a
third-party extension system are not implemented yet.

Native desktop-entry launching requires a systemd user session, with UWSM used
when it owns that session. Apps receive the session manager's environment rather
than Dojo's Cargo/devenv environment and have independent launch lifetimes.
Keyboard activation tokens and IME/
screen-reader behavior still need broader desktop testing.

Dismissing the launcher unmaps its layer surface instead of destroying it, and the next
opening maps it again with an empty search; opening on another output creates a new surface.
`devenv shell -- tools/agent-input/run-nested.sh --launcher-open` times openings in a nested
Niri, `--launcher-reuse` checks keyboard focus across hide and show, and
`--launcher-views` opens every command's view; see
[the performance notes](docs/research/dojo-launcher-performance.md).

`dojo --launcher-benchmark` reports catalogue discovery time and matcher
p50/p95/p99 for fixed queries. Use an optimized build for performance evaluation;
these measurements exclude GPUI presentation, input delivery, and IPC, and do
not establish parity with Fuzzel. The detached launch fixture test can be run with
`cargo test -- --include-ignored`; it only launches a temporary test script.

## Global shortcuts

Dojo implements the GlobalShortcuts portal, so apps can ask for system-wide
hotkeys under Niri. The first request from an app opens a small panel to choose
the keys; the choice is remembered. See [global shortcuts](docs/global-shortcuts.md)
for how it works and what it needs (the forked Niri and the running shell).

## License

Dojo is licensed under the GNU General Public License, version 3 or (at your
option) any later version; see [LICENSE](LICENSE). The shell links `niri-ipc`
from the vendored Niri fork, which is GPL-3.0-or-later, and the fork itself is
distributed under that license. The other vendored subtrees keep their own
licenses (`vendor/*/LICENSE*`, `licenses/` and the per-crate `LICENSE` files):
MIT or Apache-2.0 code such as GPUI, Cua, Hex and Keywork, which is compatible
with GPL-3.0-or-later. Crates that carry their own `license` field keep it.
