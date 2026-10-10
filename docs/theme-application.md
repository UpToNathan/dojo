# Theme application

## Current implementation

The Rust `dojo-theme` command derives paired appearances from one image snapshot,
maps extraction output to shared semantic roles, exports per-app formats, retains
the wallpaper bytes, and stores generated artifacts under
`$XDG_STATE_HOME/dojo/themes/generations/`. Activation stages a complete selection
and file links, then atomically switches `current`. The selected palette, wallpaper
and selection metadata therefore change as one internal snapshot.

This is not an atomic cross-process transaction: consumers publish/reload
independently. Every apply writes `last-apply.json` with target-level results;
`reload_requested` means a signal/event was sent, not that every pixel was verified.
Conflicts and failed reloads are reported, never disguised as successful theming.

```sh
dojo-theme preview /path/to/wallpaper.jpg /tmp/dojo-previews.html
dojo-theme presets
dojo-theme preset tokyo-night-night --targets dojo,kitty,ghostty,nvim,opencode,zed,yazi
dojo-theme generate /path/to/wallpaper.jpg --mode colorful
dojo-theme select GENERATION --appearance system --targets dojo,kitty,ghostty,nvim
dojo-theme apply /path/to/wallpaper.jpg --mode ocean --targets dojo,kitty,ghostty
dojo-theme status
dojo-theme rollback
```

Preview compares all 23 modes in both appearances, including representative UI
surfaces and ANSI swatches. It analyzes the image once, writes a new local HTML
file, and does not select/apply a theme or alter application settings.

Open the native mode picker with `dojo --theme` or the small paintbrush to the
left of the system tray. It opens on Wallpaper and orders its independent tabs
as Wallpaper, Presets, then Derive:

- **Wallpaper**: browse folders and JPEG/PNG/WebP files, preview an image, then
  explicitly **Apply wallpaper**. This preserves the existing palette, preset,
  appearance policy and targets without calling darkman. With no selected theme,
  the current config colors are retained rather than running extraction.
- **Presets**: 36 authored variants grouped by family.
- **Derive**: optional extraction attempts with dark/light previews, using the
  image chosen in Wallpaper (or the existing wallpaper if that step is skipped).
  The existing 23 modes are computed only when this tab is opened. Browsing and
  loading wallpapers do not require successful palette extraction.

The wallpaper browser starts in `~/Pictures` when present, otherwise the home
directory. Parent-folder navigation allows choosing images elsewhere. Selection
and browsing are preview-only; generation uses captured image bytes, and Apply
publishes the wallpaper and unchanged palette through the same atomic snapshot.
Browsing does not change the desktop. **Apply theme** on a derived mode follows
darkman; applying a preset explicitly calls `darkman set light` or `darkman set dark`
and pins that variant. It does not invent an opposite variant or add a schedule.
If darkman cannot switch, the new preset is not selected. External targets are
attempted by theme Apply, with per-app publication, setup and reload results shown
in a bounded scroll area. Missing setup is not reported as a successful reload.
Wallpaper-only Apply retains the existing target selection. Escape closes the picker.

Presets retain any selected wallpaper snapshot; without one they leave the
external wallpaper provider alone. Wallpaper derivation previews use the selected
snapshot, or `astro.jpg` as their initial fallback.

### Preset catalogue

| Family | Light | Dark |
| --- | --- | --- |
| Catppuccin | Latte | Frappé, Macchiato, Mocha |
| Tokyo Night | Day | Night, Storm, Moon |
| Rosé Pine | Dawn | Main, Moon |
| Kanagawa | Lotus | Wave, Dragon |
| Gruvbox | Light | Dark |
| Everforest | Light | Dark |
| Solarized | Light | Dark |
| One | Light | Dark |
| Nord | — | Nord |
| Dracula | — | Dracula |
| Ayu | Light | Dark, Mirage |
| GitHub | Light | Dark |
| Nightfox | Dayfox, Dawnfox | Nightfox, Duskfox, Carbonfox |
| Monokai Pro | — | Classic (unfiltered Pro palette) |
| Poimandres | — | Poimandres |

The bundled terminal palette data comes from the pinned iTerm2-Color-Schemes collection;
provenance, selected upstream filenames and its license are recorded in
`docs/licenses/preset-palettes.txt`. Background and ANSI colors are retained;
additional Dojo UI roles are mapped with contrast checks. This is not a claim
that every application's syntax highlighting exactly reproduces its upstream
editor colorscheme. Tokyo Night and Catppuccin now have separate, pinned authored
UI/syntax role maps, without the wallpaper derivation's automatic recoloring.
The remaining 28 presets still use terminal-palette adaptations and need the same
authored-role fidelity work; they must not be described as verified editor ports.
Authored palettes retain the author's colors, including lower-contrast comments;
foreground/surface validation has a 3:1 minimum for authored palettes rather than
silently changing them to the wallpaper derivation policy's 4.5:1 minimum.
Presets use the same exporters and ownership/reload paths as
wallpaper themes. Metadata records the preset ID rather than an extraction mode.

The `system` appearance follows darkman's control socket. Running Dojo checks mode
transitions in its background reload path; `dojo-theme watch` supplies the same
coordination without the shell. Neither mode path rebuilds Home Manager. Explicit
light/dark selection opts out of following system mode.

Colors in the user-owned config remain a fallback when no generated theme is
selected. Dojo never rewrites that config. Running shell surfaces and the native
wallpaper surfaces observe selected state; a selected generation takes precedence
over the fallback colors. Wallpaper surfaces are independent of frame enablement
and never receive pointer input. The external btrbg remains a fallback.

## Stable integration versus generated files

`theme-home-manager.nix` provisions stable terminal includes, the Yazi plugin and
flavor selection, qtct settings/packages, and the browser native-host manifest.
It is imported but **opt-in** through `dojo.themeIntegrations.enable`. It is
now enabled after explicit preset selection. Runtime coverage remains target-specific. Theme
changes after initial provisioning do not require another activation.

Generated app files have an ownership registry with content hashes. Dojo refuses
to replace unowned files, store-backed links, or outputs the user edited. JSON
settings merges are limited to theme keys and preserve unrelated values; JSONC
requires a deliberate integration rather than stripping comments/replacing it.
Terminal includes stay declaratively owned; mutable fragments stay outside Nix.

## Target contracts and outstanding setup

| Target | Implementation / acceptance status |
| --- | --- |
| Dojo | Full existing color roles, native live wallpaper loading and a three-tab picker. Theme Apply attempts every scoped target and shows individual results. |
| Niri | The Home Manager config optionally includes a mutable Dojo KDL fragment. Theme Apply maps the active palette's primary/border roles to focus-ring colors at 20% opacity and reloads Niri via IPC; wallpaper-derived themes and static presets share this exporter. The declarative fallback also uses 20% opacity. Palette RGB values are preserved. |
| Kitty/Ghostty | Stable includes are installed. Color fragments and existing-instance reload signals handle Nix-wrapped executables, not truncated `comm` names. Opacity has the separate Kitty limitation below. |
| Neovim | `colors/dojo.lua` observes selected state. Tokyo Night/Catppuccin static variants use their real upstream Neovim plugins via declarative paths in `dojo/nvim-presets.json`; derived themes use Dojo's own role mapping. Core highlights are checked headlessly. Already-open editors need a one-time config/plugin reload after this loader deployment; they are never killed or modified through external Lua RPC. |
| OpenCode | Native V2 token trees preserve explicit background/text/syntax roles without V1 migration. CLI mode is pinned to Dojo's selected appearance and republished on darkman transitions. SIGUSR2 refreshes custom theme discovery in the installed V2 TUI; server/non-TUI commands are excluded. |
| Pi | Complete 51-token theme, active user-theme publication and settings selection. Existing sessions need one initial selection of `dojo`; installed-version validation remains necessary. |
| T3 Code | Regular watched JSON with all 61 roles and paired variants. When the desktop's bundled `~/.t3/bin/t3` CLI exists, Apply calls `theme set dojo` with the configured T3 data directory so connected clients select Dojo. Without that CLI, select Dojo once in the app. Selection failures are reported separately from publication; do not symlink published JSON. |
| Zed | Paired resource plus theme-name-keyed overrides for Zed 1.19.2. Settings observation triggers theme reload without app restart. Unrelated named overrides and syntax font styles are preserved; only unchanged old Dojo flat overrides are migrated. Accent/tinted controls use explicit `info.background`/border colors rather than the default bright blue. |
| Yazi | Flavor + TextMate palette use current 26.9.1 sections and `url`/`mime` filetype selectors. The stable DDS subscriber receives `ya pub-to 0 dojo-theme --str reload` → `app:theme`; existing sessions must have loaded it. Actual installed-parser validation uses a disposable PTY, not only a TOML syntax check. |
| Delta (Zed Industries app) | Explicitly `unsupported`, not a fake successful exporter. No verified external custom-palette/live-reload interface is available. Current official docs describe synced settings with an encrypted local cache and instruct using the Settings Editor. No cache, database, account data or Git-pager configuration is touched. |
| Zen | Rust native-messaging host and a trusted dynamic theme extension calling `browser.theme.update`. Stable Zen CSS bridge supplied. Extension/profile setup is a manual one-time step; no unsigned-extension/security bypass is applied automatically. Not yet installed into the running profile. |
| Helium | The AppImage wrapper exposes `/etc/chromium/policies` read-only. Dojo writes Chromium's `BrowserThemeColor` managed policy from the active appearance's background, for both wallpaper-derived themes and authored static presets, through a root-owned, color-only helper installed by `scripts/install-system-config.sh`. The policy directory must exist before Helium starts; relaunch Helium once after installing this integration so the FHS wrapper mounts it. Later policy changes are watched live; upstream Omarchy reports about five seconds. Other Chromium builds reading that directory share the policy. Chromium derives chrome colors from the background seed; internal pages may retain stock colors. Runtime behavior on the installed Helium version still needs verification. |
| GTK3/GTK4 | Generation-named themes with toolkit-native base resources, generated roles and theme-setting notifications. CSS loads against installed GTK3/GTK4. Native settings delivery/app coverage needs runtime verification; libadwaita arbitrary palette coverage is not claimed. |
| Qt5/Qt6 | Matching qtct palettes, stable paths, Fusion style and directory-event notification. Qt6 is the provisioned session default; Qt5 uses the explicit version-specific wrapper. Existing apps without the platform plugin cannot acquire it retroactively. QML/Kvantum/application-owned graphics remain coverage work. |

Claude and Codex are intentionally excluded. No restarting applications is used as
a per-theme fallback. Neovim has the agreed Omarchy-parity/manual reload exception.

Host integration check: `python3 scripts/check-native-neovim-presets.py` starts
eight disposable headless editors with isolated XDG state, checking the native
plugin and background at startup without changing the desktop selection. The
wrapper uses native Lua `load()` entry points: nesting another `:colorscheme`
inside `:colorscheme dojo` is suppressed by Neovim's recursive-load guard.

## Rollout safeguards

Do not apply every target merely because an exporter can write a valid file. First
verify its installed serializer, one-time selection/plugin setup, existing-process
reload, appearance transition, and ownership preservation. Keep target-level
readiness visible. Choose the palette mode with the user before applying it; the
automatic `normal` test palette was disliked and is not the approved desktop theme.

The browser crash encountered during rollout is diagnosed separately in
`zen-dbus-exhaustion.md`. The VA-API-disable workaround is not a theming requirement
and must not remain as an unrelated permanent browser degradation.

## Terminal opacity and compositor fill

Kitty, its quick-access terminal and Ghostty are configured at 100% opacity.
Existing Kitty instances started without `dynamic_background_opacity` cannot
change their opacity live; fresh instances use the new setting. Do not kill
existing sessions merely to enforce opacity.

The previous 87% opacity exposed a second issue: Niri draws the focus ring as a
solid rectangle behind windows by default. The hard-coded mauve focus color
therefore tinted the entire terminal background, not just the outline. The
declarative window rule now sets `draw-border-with-background false`, validated
with `niri validate` and installed through Home Manager without restarting apps.
See upstream `niri-wm/niri/docs/wiki/Configuration:-Layout.md` (focus-ring/border)
and `Configuration:-Window-Rules.md` (draw-border-with-background).
