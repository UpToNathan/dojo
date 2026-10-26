# Theme application

## Current implementation

The Rust `dojo-theme` command applies presets or refined wallpaper-derived
palettes, exports per-app formats,
retains any selected wallpaper bytes, and stores artifacts under
`$XDG_STATE_HOME/dojo/themes/generations/`. Activation stages a complete selection
and file links, then atomically switches `current`. The selected palette, wallpaper
and selection metadata therefore change as one internal snapshot.

This is not an atomic cross-process transaction: consumers publish/reload
independently. Every apply writes `last-apply.json` with target-level results;
`reload_requested` means a signal/event was sent, not that every pixel was verified.
Conflicts and failed reloads are reported, never disguised as successful theming.

```sh
dojo-theme presets
dojo-theme preview /path/to/wallpaper.jpg /tmp/previews.html
dojo-theme apply /path/to/wallpaper.jpg --mode ocean --appearance dark
dojo-theme preset tokyo-night-night --targets dojo,kitty,ghostty,nvim,opencode,yazi
dojo-theme status
dojo-theme rollback
```

Wallpaper-derived palettes use the refined recipe across all 23 modes. Status
colors fit into semantic hue ranges and meet contrast targets on their actual
surfaces in dark and light appearances. Old stored generations retain their
original colors when selected or rolled back.

Open the native theme picker with `dojo --theme` or the small paintbrush to the
left of the system tray. It opens on Wallpaper and offers Wallpaper, Presets, and Derive:

- **Wallpaper**: **Choose image…** opens the configured desktop FileChooser
  portal for a JPEG/PNG/WebP, previews the chosen image, then explicitly
  **Apply wallpaper**. This preserves the existing palette, preset,
  appearance policy and targets without calling darkman. With no selected theme,
  the current config colors are retained rather than running extraction.
  Below it, **Each monitor** (the default) or **Span all monitors** chooses how the
  wallpaper sits on the displays. Like an image, the layout is preview-only until
  **Apply wallpaper**, which saves it to `~/.local/state/dojo/themes/wallpaper-layout`
  (`fill` or `span`). That file lives outside the theme generations and survives theme
  changes. Changing only the layout and pressing Apply saves just the layout; the
  wallpaper image and theme are left alone.
  Under **Choose image…**, the style buttons (Contours, Strata, Ripples) draw a
  *procedural wallpaper*: a recipe each monitor draws at its own resolution, so it is
  sharp on rotated and high-density monitors and continuous across a span. Drawing one
  previews **Span all monitors**, and with no saved layout it spans. **Theme colours**
  (the default) borrow the selected theme's background and accents, **Random colours**
  take them from the seed, and **Paste palette** reads two or more `#rrggbb` colours from
  the clipboard. Colours are made dark enough to sit behind windows. The recipe is kept
  as `wallpaper-recipe.json` beside the retained `wallpaper` (a PNG of the whole canvas
  that derivation, the lock screen and the login screen read like any image). Applying a
  preset keeps it; choosing an image replaces it; it does not follow later theme changes.
  `dojo-theme procedural preview|apply STYLE` does the same from a shell.
- **Presets**: 41 variants grouped by family.
- **Derive**: previews all 23 refined modes from the chosen wallpaper. Select
  Dark or Light, then Apply theme to publish that exact mode and appearance.
  Opening Derive or selecting a mode only changes the preview.

Dojo delegates filesystem browsing to the system picker rather than embedding
its own file tree. A cancelled request leaves the selection unchanged. Selection
is preview-only; Apply publishes the wallpaper and unchanged palette through the
same atomic snapshot. Choosing a file does not change the desktop. Applying a
preset explicitly calls `darkman set light` or `darkman set dark`
and pins that variant. It does not invent an opposite variant or add a schedule.
If darkman cannot switch, the new preset is not selected. External targets are
attempted by theme Apply, with per-app publication, setup and reload results shown
in a bounded scroll area. Missing setup is not reported as a successful reload.
Wallpaper-only Apply retains the existing target selection. Escape closes the picker.

Presets retain any selected wallpaper snapshot; without one they leave the
wallpaper unchanged. Dojo restores the selected snapshot after login and
retains its decoded image until a replacement is ready. There is no hard-coded
Astro startup provider.

The Omarchy presets add palette data only. They do not bundle or select Omarchy
wallpapers.

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
| Flexoki | Light | — |
| Miasma | — | Miasma |
| Matte Black | — | Matte Black |
| Osaka Jade | — | Osaka Jade |
| Ristretto | — | Ristretto |

The bundled terminal palette data comes from the pinned iTerm2-Color-Schemes collection;
provenance, selected upstream filenames and its license are recorded in
`docs/licenses/preset-palettes.txt`. Background and ANSI colors are retained;
additional Dojo UI roles are mapped with contrast checks. This is not a claim
that every application's syntax highlighting exactly reproduces its upstream
editor colorscheme. Tokyo Night and Catppuccin now have separate, pinned authored
UI/syntax role maps, without the wallpaper derivation's automatic recoloring.
The remaining 33 presets still use terminal-palette adaptations and need the same
authored-role fidelity work; they must not be described as verified editor ports.
Authored palettes retain the author's colors, including lower-contrast comments;
foreground/surface validation has a 3:1 minimum for authored palettes rather than
silently changing them to the wallpaper derivation policy's 4.5:1 minimum.
Presets use the same exporters and ownership/reload paths as
wallpaper themes. Metadata records the preset ID rather than an extraction mode.

Selection mappings and the repeatable all-preset audit are documented in
[the preset export audit](theme-preset-audit.md). Apps without a selected-text
foreground use compatible surfaces rather than a terminal selection fill.

The `system` appearance follows darkman's control socket. Running Dojo checks mode
transitions in its background reload path; `dojo-theme watch` supplies the same
coordination without the shell. Neither mode path rebuilds Home Manager. Explicit
light/dark selection opts out of following system mode.

Colors in the user-owned config remain a fallback when no generated theme is
selected. Dojo never rewrites that config. Running shell surfaces and the native
wallpaper surfaces observe selected state; a selected generation takes precedence
over the fallback colors. Wallpaper surfaces are independent of frame enablement
and never receive pointer input. Selected wallpapers use the Bottom layer. Dojo
loads a complete replacement before swapping the image and retires the previous
GPU image after painting. `scripts/install-wallpaper-owner.py` retires the legacy
Astro startup entry with a validated scoped config and timestamped backup. The paintbrush and launcher buttons open their windows on
the display they were clicked on.

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
| Niri | The Home Manager config optionally includes a mutable Dojo KDL fragment. Preset Apply maps the active palette's primary/border roles to focus-ring colors at 20% opacity and reloads Niri via IPC. The declarative fallback also uses 20% opacity. Palette RGB values are preserved. |
| Kitty/Ghostty | Stable includes are installed. Color fragments and existing-instance reload signals handle Nix-wrapped executables, not truncated `comm` names. Opacity has the separate Kitty limitation below. |
| Neovim | `colors/dojo.lua` observes selected state. Tokyo Night/Catppuccin static variants use their real upstream Neovim plugins via declarative paths in `dojo/nvim-presets.json`; other presets use Dojo's role mapping. Core highlights are checked headlessly. Already-open editors need a one-time config/plugin reload after this loader deployment; they are never killed or modified through external Lua RPC. |
| OpenCode | Native V2 token trees preserve explicit background/text/syntax roles without V1 migration. CLI mode is pinned to Dojo's selected appearance and republished on darkman transitions. SIGUSR2 refreshes custom theme discovery in the installed V2 TUI; server/non-TUI commands are excluded. |
| Pi | Complete 51-token theme, active user-theme publication and settings selection. Existing sessions need one initial selection of `dojo`; installed-version validation remains necessary. |
| T3 Code | Regular watched JSON with all 61 roles and paired variants. When the desktop's bundled `~/.t3/bin/t3` CLI exists, Apply calls `theme set dojo` with the configured T3 data directory so connected clients select Dojo. Without that CLI, select Dojo once in the app. Selection failures are reported separately from publication; do not symlink published JSON. |
| Yazi | Flavor + TextMate palette use current 26.9.1 sections and `url`/`mime` filetype selectors. The stable DDS subscriber receives `ya pub-to 0 dojo-theme --str reload` → `app:theme`; existing sessions must have loaded it. Actual installed-parser validation uses a disposable PTY, not only a TOML syntax check. |
| Zen | Rust native-messaging host and a trusted dynamic theme extension calling `browser.theme.update`. Stable Zen CSS bridge supplied. Extension/profile setup is a manual one-time step; no unsigned-extension/security bypass is applied automatically. Not yet installed into the running profile. |
| Helium | Stock Helium watches Dojo's managed `BrowserThemeColor` policy. Apply uses the semantic background as the seed, keeping it subdued, and the browser uses its normal desktop appearance handling. Chrome derives the other browser colors, so they do not match T3's full palette exactly. No custom browser build, extension reload or debugging connection is required. The policy directory must be exposed by the Home Manager launcher. |
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

## Yazi file integration

Yazi is the shared folder association, FileManager1 reveal service, and FileChooser portal backend. `yazi-home-manager.nix` owns this routing. `scripts/install-yazi-integration.sh` can activate a scoped artifact built with `scripts/check-yazi-module.nix`, with backups, without deploying unrelated desktop changes. GTK native dialogs use the portal for newly launched apps; Qt's existing `standard_dialogs=xdgdesktopportal` setting uses the same backend. Apps with hard-coded file widgets can bypass system interfaces and require app-specific changes. Open, save, folder selection and multiple-selection are supported; cancellation publishes no paths.

### Shell foreground pairs

Dojo exports and loads `primary_foreground`, `selection_foreground`, `surface`,
and `surface_raised` alongside its original six color roles. Controls filled
with `primary` or `background_highlight` use the corresponding foreground.
Authored pairs remain exact. Existing six-role config and saved generations
remain accepted: missing surfaces use background and highlight, respectively;
a missing paired foreground uses normal text when it meets 4.5:1 against the
fill, otherwise whichever of black or white has higher contrast.

### Lock screen wallpaper

Applying a generation that retains a wallpaper also renders
`lock-wallpaper.png` (480 px wide, three box-blur passes, 45% dim) next to
`palette.json`, so the locker loads it from `current/` and never blurs live.
The render runs once per generation inside the apply lock, before `current`
switches. Generations without a wallpaper have no such file. A render failure
does not fail the apply; it is recorded in `last-apply.json` as a `lock-wallpaper`
result with status `blocked`, and the locker falls back to a flat palette colour.
The shell's closing and the locker read the same file, or draw the same flat
colour when it is absent, so the wallpaper never changes at the handoff.

A theme applied before the lock wallpaper existed has none. `dojo-theme lock-wallpaper`
renders it for the selected generation and appearance and links it into the
current activation. It publishes no target, signals nothing, makes no new
activation and leaves `current` where it is; it prints what it did as JSON
and is idempotent. A generation without a retained wallpaper gets no file
(`path` is `null`).

### Spanning monitors

With the `span` layout one image covers the bounding box of all enabled monitors,
scaled to cover it and centred. Each wallpaper window shows the part of the image
behind its monitor: Niri supplies every monitor's logical position and size, so
scale, rotation and gaps between monitors are respected (a gap hides the picture
behind it, as with one picture seen through separate windows). Dojo re-slices when
monitors are rearranged, rotated or connected. If Niri cannot be queried or the
image header cannot be read, each monitor falls back to filling. Spanned slices are
prepared per window and are not stored in the wallpaper cache, which holds whole
monitor-sized images. The lock screen's own wallpaper is not spanned.

### Monitor wallpaper preparation

The original wallpaper snapshot stays intact. Each wallpaper window prepares
a filtered cover crop for its physical pixel size, using its viewport and
display scale. Preparation runs off the UI thread, refreshes after output
size/scale changes, and preserves animation frame delays. The previous image
stays visible until the new image is ready; superseded preparation is ignored.
