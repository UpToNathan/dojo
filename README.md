> [!IMPORTANT]
> Remove this line to confirm you've reviewed this PR before submitting.

# Dojo

## Desktop configuration

This repository is the source for the desktop's devenv inputs and definition,
Home Manager module, Neovim and Rush configs, wallpaper, and custom PAM, Nix,
1Password, systemd, tmpfiles, and udev configuration. `~/.config/devenv` keeps
symlinks to maintained files here; its `.devenv` directory is local generated
cache/state and is not part of the source configuration.

Run `devenv shell` or `devenv eval ...` from this directory. Run
`scripts/install-system-config.sh` to link the maintained system files into
place; it preserves replaced files as timestamped backups. See
`system-config/README.md` for tracked inputs versus generated runtime state.

Clipboard history and persistence are separate services: Clipvault stores past
entries, while `wl-clip-persist` retains the regular clipboard in memory after
the source app closes. Both run with the graphical session; primary selection
is unchanged. To verify in a live Wayland session, run
`python3 scripts/check-clipboard-persistence.py` (replaces the current clipboard).

## Project

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
AI clients. Build with `devenv shell -- cargo build --bin dojo-agent`; see
[client setup](docs/agent-input.md#connect-an-ai-client). Each server retains its
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

Edits reload within approximately 500 ms without restarting. Invalid edits keep
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
cargo run --bin dojo-palette -- astro.jpg --mode normal --dark
cargo run --bin dojo-palette -- astro.jpg --mode ocean --light
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

## Applying generated themes

`dojo-theme` derives semantic roles and paired appearances, retains wallpaper
snapshots, exports application formats and coordinates reloads without rewriting
the user config. Run `dojo-theme --help`; see `docs/theme-application.md` for
target readiness, opt-in Home Manager setup, live-update contracts and rollback.

GNOME Keyring's Secret Service component is started with the graphical session
for applications such as Zed and Delta. They discover `org.freedesktop.secrets`
on the user session bus; no extra secrets-provider CLI argument is required.
Starting the daemon does not bypass keyring locking or unlock prompts.
GCR's SystemPrompter and PrivatePrompter D-Bus activation files are also
provisioned: the daemon alone cannot create or unlock a keyring if its prompt
service is absent. Enter keyring passwords only in the native dialog.

Use `dojo-theme preview IMAGE OUTPUT.html` before choosing and applying a palette.
The paintbrush beside the system tray (or `dojo --theme`) opens Wallpaper,
Derive and Presets tabs. Wallpaper changes can keep the existing palette;
theme Apply shows per-app setup/reload results. Static presets switch darkman
to their authored light/dark mode. Tokyo Night/Catppuccin use upstream native
Neovim plugins; other presets still need authored-role fidelity validation.
The automatic normal-mode test palette is not the approved desktop theme.
See `docs/zen-dbus-exhaustion.md` for the separate wluma reply-accumulation fix.

## Application launch lifetime

Dojo parses desktop-entry command arguments itself; it does not use `gio` or a
shell interpreter. In an active UWSM session it delegates lifecycle/environment
handling to `uwsm app -t service`. Otherwise it uses a separate systemd user scope.
Apps have independent process groups/cgroups and journal logging and survive a
Dojo restart. D-Bus-only entries use explicit activation; timeout is not treated
as permission to spawn a second instance. Use `dojo --quit` for shell shutdown.

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
Extension providers, desktop actions, PATH-only commands and history are not
implemented yet.

Native desktop-entry launching requires a systemd user session, with UWSM used
when it owns that session. Apps receive the session manager's environment rather
than Dojo's Cargo/devenv environment and have independent launch lifetimes.
Keyboard activation tokens and IME/
screen-reader behavior still need broader desktop testing.

`dojo --launcher-benchmark` reports catalogue discovery time and matcher
p50/p95/p99 for fixed queries. Use an optimized build for performance evaluation;
these measurements exclude GPUI presentation, input delivery, and IPC, and do
not establish parity with Fuzzel. The detached launch fixture test can be run with
`cargo test -- --include-ignored`; it only launches a temporary test script.
