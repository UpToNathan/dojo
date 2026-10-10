# Wallpaper-connected theming: Omarchy and Aether compatibility

Research date: 2026-10-09. **Research only:** no desktop configuration, application restart, implementation, or commit. Read `AGENTS.md`, `GLOSSARY.md`, and `docs/agents/domain.md`; no ADR files were found. This report does not define Dojo configuration values or a schema.

## Outcome

**Prefer data-format compatibility, not running Omarchy's application machinery on Dojo.** Import curated palettes and wallpapers, optionally use Aether as an external editor/generator, and retain a Dojo-owned application boundary for the desktop frame, shell surfaces, wallpaper, and explicitly supported app integrations. This is a proposal, not an existing Dojo capability.

The linked gallery currently uses **Aether protocol links to object-storage palettes and images**, not a catalogue of Git repository install commands. Omarchy repository import and gallery consumption therefore need distinct acquisition paths. Aether really does have a standalone path, but “Apply” is not a side-effect-free palette export: editor integration and custom hooks can run. [G1][G2][A1][A2]

## Source baseline

Inspected public default-branch source, pinned below; these are source snapshots, **not assertions about the user's installed releases**:

| Project | Inspected revision |
| --- | --- |
| `omacom/omarchy` | `61263219b55250b79a7332d4d8ab3904b8cdecbb` |
| `omacom/aether` | `29d07b1a8f474c77bf02aece948373f40abed105` |
| `bjarneo/omarchy-themes` | `a455886e43e444d8d69faa8744bd5d8a4c27e1b9` |
| `nix-community/home-manager` | `6b88c12cc6d234de4888f5d21076fb11199d0844` |

The report separates **verified source facts**, **uncertainties**, and **proposals**. Citations resolve to primary source paths at these revisions unless marked live documentation.

## 1. Verified: Omarchy's theme contract

### Palette, application files, and backgrounds

- Themes live in the user root `~/.config/omarchy/themes` and the stock root `$OMARCHY_PATH/themes`. Applying overlays the user's theme on the matching stock theme. The manual describes `/usr/share/omarchy/themes` as the stock location. [O1][O2]
- `colors.toml` is now the central palette. A stock example contains `mode`, semantic `accent`, `selection`, `muted`, background/foreground shades, ordinary named colors, and bright named colors. The resolver accepts legacy `color0`–`color15` and short aliases such as `bg`, `dark_bg`, and `fg`, derives missing colors, and gives canonical names precedence. This is not simply a mandatory 16-element array. [O3][O4]
- The actual resolver is a constrained line-oriented parser, not a general-purpose TOML implementation. A compatibility reader should explicitly target supported palette declarations rather than promise arbitrary TOML equivalence. This conclusion follows from the parser's key/value loop and character restrictions. [O4]
- Per-app files can override generated output; templates fill only missing files. User `~/.config/omarchy/themed/*.tpl` templates take precedence over built-ins in `default/themed`. Syntax includes `{{ background }}`, `_strip`, `_rgb`, and current mix/gradient functions; `shell.*.toml` can override sections of generated `shell.toml`. [O5]
- A legacy theme without `colors.toml` can have a palette extracted from `alacritty.toml`. Repository themes are treated differently from locally authored themes: top-level Lua, terminal configurations, and `vscode.json` are excluded from staging, and terminal colors can be reconstructed without staging the terminal's executable settings. Symlinks in repository themes are not followed during staging. This is an upstream trust policy, not proof that all remaining arbitrary app files are safe for a different consumer. [O2]
- Background selection searches both `~/.config/omarchy/backgrounds/<theme>/` and the staged theme's `backgrounds/`, non-recursively. It handles listed still-image and video extensions, remembers the selected background per theme, and cycles when reapplying the current theme. Preview/unlock images and `icons.theme` are additional optional theme assets. [O1][O2]

### Installation and active paths: important correction

`omarchy-theme-install` validates the Git URL and normalized repository-derived name, removes an existing installed directory of that name, clones into `~/.config/omarchy/themes/<name>`, and immediately applies it. It is **not** a download-only or rollback-preserving import API. [O6]

At this revision, `~/.local/state/omarchy/current/theme` is a **generated directory**, not a symlink selecting an installed repository. Application configs include stable paths inside that directory (Kitty `include`, Ghostty `config-file`, Alacritty `general.import`); the active wallpaper **is** a symlink at `current/background`, while `current/theme.name` stores the name. Older `~/.config/omarchy/current` references exist in migration code and Aether's backward-compatible reader. Avoid designing against the assumption that changing one theme symlink reproduces current Omarchy behavior. [O2][O7][O8][A3]

### Apply, reload, and transaction limits

Omarchy serializes the staging/current critical section with `flock`, generates into `next-theme`, then does `rm -rf current/theme` followed by `mv next-theme current/theme`. Although its comment calls this an atomic swap, the source contains a missing-path interval; it is not an atomic replacement of an existing directory. Name, background link, shell IPC, and later app hooks are separate operations. No whole-desktop rollback transaction is established by this script. [O2]

The running Omarchy shell receives palette and shell payloads over IPC; app-specific hooks then run in parallel after the lock is released. Terminal reload touches Alacritty's config, sends Kitty `SIGUSR1`, and Ghostty `SIGUSR2`. Other hooks include compositor, browser, editors, GNOME settings, and `theme-set` hooks. These are Omarchy-specific orchestration, not a Niri/Dojo API, and app retint completion is not equivalent to a globally atomic visual change. [O2][O9]

### Light and dark

Mode resolution precedence is `mode`, legacy `theme_type`, adjacent `light.mode`, background-channel-sum inference, then dark. The documented modern form is `mode = "light"`. GNOME integration writes `prefer-light`/`prefer-dark`, Adwaita/Adwaita-dark, and the chosen icons. [O1][O4][O10]

The examined directory/application contract represents **one active palette/mode at a time**, not an explicit paired light/dark bundle. Separately named light themes exist, but pairing, transition policy, and a scheduled mode authority must not be inferred from a mode marker. [O2][O3][O4]

## 2. Verified: Aether standalone and interchange

### What standalone really means

The README advertises standalone Linux use and wallpaper extraction/editing. The standalone guide places generated output in `~/.config/aether/theme/` and describes manual includes and wallpaper setup. Actual mode detection checks whether the public `omarchy` command exists on Linux; it does not validate a complete Omarchy desktop. A shim named `omarchy` would therefore change execution mode and should not be introduced just to satisfy a gallery link. [A1][A2][A4]

Source is more precise than the guide's “not automatically applied system-wide”: standalone `ApplyTheme` writes built-in templates, updates `light.mode`, applies selected editor integrations, and processes custom apps. A custom app's `config.json` names a template and destination; processing creates a destination symlink and launches optional `post-apply.sh` asynchronously. Standalone does not invoke Omarchy's shell/compositor/background machinery, but **can change application files and execute user-configured scripts**. [A2][A5]

`GenerateOnly` is the safer generation boundary: its source explicitly avoids custom-app destination symlinks, service restarts, and Omarchy activation, while generating files, background assets, and optional VS Code extension output. CLI docs expose `--generate ... --no-apply --output ...`; these are integration possibilities, not commands executed in this research. [A2][A6]

### Blueprint JSON versus exported theme folder

These are distinct artifacts:

| Artifact | Verified upstream content/behavior |
| --- | --- |
| Blueprint JSON | Top-level `name`, `palette`, `adjustments`, `appOverrides`, `settings`, optional `iconTheme`, and `timestamp`. Palette fields include `colors`, wallpaper path/URL/source, blur, additional images, `lightMode`, three-valued `mode`, locks, extended colors, and native colors. No explicit schema-version field appears in the examined Go model. [A7] |
| Imported `colors.toml` | Converted into blueprint palette data, preserving explicit extended colors, native colors, and parsed mode; incomplete neutral slots receive defaults before validation. Import is therefore not byte-preserving storage of arbitrary TOML. [A8] |
| Theme-folder export | GUI exports `omarchy-<slug>-theme` containing generated app files and assets. On Omarchy it selects native v4 generation; standalone selects general generation. This is not a blueprint JSON or a theme ZIP. Optional installation uses an Omarchy theme-root symlink. [A9] |
| Favorites ZIP | A separate wallpaper-collection feature, not the theme-folder interchange contract. [A1] |

Blueprint locks accept both index arrays and older positional booleans. The docs' example is abbreviated: the Go model is authoritative for current native colors, three-valued mode, icon choice, and additional images. Blueprint-local wallpaper paths are not self-contained portable assets. [A7][A10]

Aether's template syntax is **not Omarchy's**: `{background}`, `{color0.strip}`, `.rgb`, `.rgbs`, `.rgba[:alpha]`, and `.yaru`, versus Omarchy's double braces and underscore modifiers. Sharing palettes is substantially easier than sharing template engines verbatim. [A5][A11][O5]

### Native Omarchy integration and recovery

Aether's native generator stages a sibling directory, marks ownership with `.aether-managed`, refuses unmanaged nonempty output, backs up the old directory, installs the generated directory, and retains the backup until activation succeeds. Activation calls the public `omarchy theme ...` CLI; recovery snapshots the previous wallpaper outside Omarchy's replaceable runtime directory. [A2][A4]

This is useful precedent, **not a complete transaction guarantee**: directory backup/install are sequential renames; the activation wrapper explicitly says it does not serialize transactions; wallpaper restoration and bundle restoration do not establish rollback of all already-run Omarchy hooks or its copied runtime palette. Standalone writes files individually, with atomic per-file helpers, not a single bundle-level commit. [A2][A4][A12]

## 3. Verified: the linked gallery's compatibility requirements

The gallery source builds `wallpapers.js` from `wallpapers.json`, prefixes asset paths with an externally supplied base URL, and deploys with `https://wallpapers.hel1.your-objectstorage.com`. Its index maps wallpaper paths to dimensions, tone, title, tags, thumbnails/medium assets, and variant metadata. At the inspected snapshot a local read counted 3,088 entries; variant keys are `palette`, `gruvbox`, `nord`, `material`, and `aether` (UI labels Palette/Warm/Cool/Material/Aether). These names are palette variants, **not paired light/dark variants**. [G1][G2][G3]

`res_min=5K` is a lower-bound **width-tier** filter: width ≥4,800 gets 5K, width ≥7,000 gets 8K+, and the range includes higher tiers. It does not guarantee 5,120×2,880, aspect ratio, safe decoder size, or panel fit. [G1]

The two variant actions construct:

- Edit: `aether://apply?colors=<absolute colors_toml URL>&wallpaper=<original wallpaper URL>&edit=true`.
- Apply: the same assets with `silent=true&as_omarchy_theme=<variant slug>`.

The gallery uses the original index key as the wallpaper URL rather than automatically selecting the per-variant `background` field. `neovim_lua` is metadata, not downloaded by these action URLs. There is no Git clone in this button path. [G1][G3]

Aether supports this protocol in source: validates/downloads assets, permits silent apply, and named native installation requires Omarchy. `edit=true` wins over native install. Consequently **installing Aether standalone does not by itself make the gallery's named Apply action work on Dojo**; its Edit action is the plausible external-editor route. A native gallery adapter could instead consume the index and palette/image URLs without claiming to be Omarchy or taking over the system-wide `aether://` association. [A13][A14][A4]

The gallery cannot observe whether Aether accepted an Apply link; its toast acknowledges the click, not successful application. Dojo should report its own actual transaction result if it offers equivalent actions. [G1][G4]

## 4. Verified constraints and proposed Dojo improvements

### Home Manager/Nix ownership

**Verified local context:** `home-manager.nix` owns the Neovim directory, Rush config, Ghostty config, Kitty settings, Niri settings, and portal selection. It enables darkman with `portal = true`, disables GeoClue guessing, binds a Niri shortcut to `darkman toggle`, and launches btrbg with a fixed wallpaper at login. These are repository facts, not verification of the currently activated home generation. [D1]

Home Manager links managed files from generated/store-backed paths, checks collisions before activation, and relinks during generation activation. Aether's atomic writer resolves destination symlinks and writes the referent; custom-app installation creates destination symlinks. Writing into Nix-owned locations can fail, conflict with ownership, or be replaced on the next activation. Backups or `force` do not solve competing authorities. [H1][A12][A5]

**Proposal:** keep declarative app config owned by Home Manager, with explicit supported includes pointing to mutable generated theme fragments outside the store. Do not replace the whole Kitty/Ghostty/Niri/Neovim configuration to change colors. Audit each adapter's real destination and existing parent symlinks before enabling it; unknown ownership should block that adapter, not silently overwrite. Include syntax precedent exists in Omarchy's terminal configs and Aether's standalone guide, but exact Dojo/Niri/editor wiring still requires app-specific verification. [O7][A15]

### Mode authority and darkman

Home Manager's darkman module can install mode scripts (including a unified script receiving `dark`/`light`) and legacy `dark-mode.d`/`light-mode.d` scripts; it runs a graphical-session-bound D-Bus service. Dojo's current portal configuration explicitly chooses darkman as Settings provider. [H2][D1]

**Proposal:** retain darkman as the mode authority; a later integration can observe or respond to mode changes to choose the corresponding curated palette/wallpaper variant. Do not make a theme picker, Aether, and darkman independently write competing appearance preferences. Treat a wallpaper's light/dark siblings as curated selections; do not invert a palette or assume all wallpapers have both modes. Decide explicitly what happens when a compatible counterpart is absent. The inspected upstream formats do not supply a universal paired-variant contract. [O4][A7][G3]

**Uncertainty:** darkman's own hosted manual timed out during this research, so its exact installed D-Bus/portal behavior and timing are not independently established here. The Home Manager source and local portal declaration establish intended wiring, not runtime success or sunrise/sunset scheduling with the user's current location settings. [H2][D1]

### Transaction, rollback, and live reload

**Proposal:** validate/download/render into an immutable generation first; serialize commits; atomically replace a single active-generation pointer; retain the prior generation; then perform application-specific reloads. Record palette, wallpaper, and mode as the same committed selection. Persist recovery information before side effects and distinguish “generation committed” from “all consumers reloaded.” Coalesce preview/live-apply events and give stale work a generation identity so older downloads/reloads cannot win. These are improvements over the separate operations observed upstream, not features claimed for Dojo. [O2][A2][A4]

**Proposal:** rollback must reselect the old palette **and** old wallpaper and reload already-updated consumers. Arbitrary hooks are not reversible transactions; disable imported hooks and represent irreversible/failed integrations explicitly. Neither an atomic file rename nor a shell success toast proves simultaneous multi-process application. [A5][A12][G4]

**Proposal:** update Dojo's own desktop frame and shell surfaces in-process; use verified include/reload contracts per app and the actual Niri/btrbg mechanisms rather than copying Hyprland hooks. The terminal signals above are source evidence for Omarchy's adapters, not a mandate to restart user apps, nor confirmation for every installed version. [O9][D1]

### Asset safety and provenance

Aether's downloader enforces public HTTPS, rejects credentials/local/private destinations, checks redirects and dial-time DNS addresses, disables proxies, bounds documents to 1 MiB and images to 50 MiB, and validates decoded dimensions (≤16,384 per side and ≤40 million pixels). Cache names hash the **URL**, not the downloaded bytes, and existing regular cached files are reused. This is bounded retrieval, not signed provenance or content-integrity pinning. [A16]

**Compatibility caveat:** the gallery's first source entry is 10,667×6,000, already over Aether's 40-million-pixel limit. A local scan of the inspected index found 34 entries whose declared width×height exceeds that limit. This demonstrates a metadata-level incompatibility candidate; actual bytes were not downloaded or decoded, so individual failures remain unverified. [G3][A16]

The examined gallery's top-level metadata fields contain no author/license/content-hash/repository fields; a nested variant `source` is an internal palette path, not an image-rights statement. Therefore do not equate “publicly downloadable,” gallery membership, or an Omarchy theme repository's software license with wallpaper redistribution permission. Asset authorship/licenses remain unresolved. [G3]

**Proposal:** preserve acquisition URL, repository commit where applicable, content digest, retrieval date, and any provided author/license with each asset; pin catalogue snapshots separately from mutable remote assets. Validate actual image dimensions/format, restrict symlinks/path traversal and archive expansion, limit bytes/pixels/disk use, and make missing provenance visible. Import palette/image data by default, never repository Lua, shell hooks, terminal launch settings, or editor extensions. Do not silently downsample a rejected high-resolution image without communicating that choice. [O2][O6][A16]

## 5. Integration choices

| Choice | Assessment / proposal |
| --- | --- |
| Reuse Aether standalone | Fastest route to an existing wallpaper editor/extractor and blueprint library. Prefer generation-only output or explicit editor handoff, with Dojo-owned application. Must avoid standalone Apply's editor/custom-hook side effects and cannot promise the gallery's Omarchy-specific Apply links. [A1][A2][A13] |
| Import formats | Best initial compatibility seam: consume Omarchy palette + backgrounds, gallery asset metadata, and optionally blueprint JSON. Preserve semantic/native colors and curated mode, do not reduce everything to extraction output. Templates and arbitrary per-app files should be opt-in local integrations, not imported execution. [O3][O4][A7][A8][G3] |
| Native compatible Dojo editor | Best eventual integrated experience if editing within shell surfaces is important, but entails maintaining color extraction, image tooling, schema migrations, catalogue parsing, validation, and reload adapters. Proposed, not required for compatibility; Aether can remain an external authoring tool. [A1][A7][A11] |
| Run Omarchy install/apply directly | Poor fit: immediately modifies installed themes, uses Omarchy shell/Hyprland/session commands and hooks, and conflicts with the existing Nix-owned desktop configuration. Format compatibility does not require command/runtime compatibility. [O2][O6][D1] |

**Recommended scope:** first establish read-only import/preview and a trusted local generation/application boundary; evaluate optional Aether handoff before duplicating its editor. Define a supported compatibility subset rather than claiming every historical Omarchy theme, Aether template, or gallery action works unchanged. This recommendation intentionally does not prescribe a Dojo settings schema.

## 6. Remaining uncertainties / validation needed later

- Installed Aether/Omarchy/Home Manager versions may lag these revisions. No binaries or desktop integrations were executed.
- Exact parser parity across all historical repositories, native gradients, and app overrides needs a fixture corpus; current Aether and Omarchy have separate parsing/generation code, not a published versioned interoperability specification. [O4][A8]
- Gallery object-storage contents, original-image rights, metadata accuracy, and asset availability were not audited. The deployed page was fetched; no wallpaper was applied. [G2][G3][G4]
- Niri/btrbg live wallpaper and fragment reload behavior, terminal version-specific signals, and real darkman scheduling need isolated validation before an implementation chooses adapters. The local repository declarations alone do not establish these behaviors. [D1][O9][H2]
- Aether docs sometimes simplify source behavior (standalone side effects, blueprint fields, “atomic” terminology). The cited implementation should take precedence for those distinctions. [A2][A7][A15]

## Primary sources

[O1]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/manual/43-making-your-own-theme.md
[O2]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-theme-set
[O3]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/themes/catppuccin/colors.toml
[O4]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-theme-color
[O5]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-theme-set-templates
[O6]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-theme-install
[O7]: https://github.com/omacom/omarchy/tree/61263219b55250b79a7332d4d8ab3904b8cdecbb/config
[O8]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/migrations/1781043107.sh
[O9]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-restart-terminal
[O10]: https://github.com/omacom/omarchy/blob/61263219b55250b79a7332d4d8ab3904b8cdecbb/bin/omarchy-theme-set-gnome
[A1]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/README.md
[A2]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/theme/writer.go
[A3]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/omarchy/system.go
[A4]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/omarchy/native.go
[A5]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/template/custom_apps.go
[A6]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/docs/cli.md
[A7]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/blueprint/model.go
[A8]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/blueprint/import.go
[A9]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/app.go#L1145-L1268
[A10]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/docs/blueprints.md
[A11]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/template/engine.go
[A12]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/platform/fileutil.go
[A13]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/docs/web-handler.md
[A14]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/cli/url_handler.go
[A15]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/docs/standalone.md
[A16]: https://github.com/omacom/aether/blob/29d07b1a8f474c77bf02aece948373f40abed105/internal/wallpaper/download.go
[G1]: https://github.com/bjarneo/omarchy-themes/blob/a455886e43e444d8d69faa8744bd5d8a4c27e1b9/app.js
[G2]: https://github.com/bjarneo/omarchy-themes/blob/a455886e43e444d8d69faa8744bd5d8a4c27e1b9/.github/workflows/deploy.yml
[G3]: https://github.com/bjarneo/omarchy-themes/blob/a455886e43e444d8d69faa8744bd5d8a4c27e1b9/wallpapers.json
[G4]: https://bjarneo.github.io/omarchy-themes/?res_min=5K
[H1]: https://github.com/nix-community/home-manager/blob/6b88c12cc6d234de4888f5d21076fb11199d0844/modules/files.nix
[H2]: https://github.com/nix-community/home-manager/blob/6b88c12cc6d234de4888f5d21076fb11199d0844/modules/services/darkman.nix
[D1]: ../../home-manager.nix
