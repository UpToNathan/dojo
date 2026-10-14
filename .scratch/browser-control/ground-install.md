# Browser Control installation grounding

Read-only source inspection on 2026-10-13. The only workspace write in this task is this note. No implementation, browser/profile changes, credential files, or tests were read or run. The existing dirty checkout was preserved.

## Sources and pin

The reviewed checkout is `/tmp/dojo-browser-control-research-b4e4238`, commit `b4e4238742fa73daabfb664f1108113e9577849d`. Its package is `@opencode-ai/browser-control` version `0.8.4`, MIT licensed. `package.json` requires Node >=22.19.0 and names pnpm 11.20.0. `pnpm-lock.yaml` uses lockfile version 9.0. The extension has its own manifest version, 0.0.25, and minimum Chrome 120.

Dojo sources reviewed include `GLOSSARY.md`, `docs/agents/domain.md`, `system-config/README.md`, `devenv.nix`, `devenv.yaml`, `devenv.lock`, the relevant Home Manager modules, the agent setup registry and writer, browser callers, and agent integration scripts. There is no ADR directory or existing browser-control spec in this checkout at inspection time.

## Current ownership and runtime flow

`devenv.nix` receives `pkgs` and `inputs`, provides development packages, and imports `home-manager.nix` under `machines.venus.home-manager`. `devenv.yaml` pins external flakes, and `devenv.lock` resolves them. Home Manager installs the graphical apps and command line programs. The documented deployment command is `devenv machines deploy venus`.

Home Manager installs OpenCode and Pi from separate pinned flake inputs. The `t3` command is a wrapper around `/home/nathan/.local/opt/t3code`, an installed Electron bundle outside Nix. `t3-home-manager.nix` installs this wrapper only. T3 Desktop owns its backend and its existing `~/.t3/userdata`; starting another backend or altering the local environment is outside browser setup.

OpenCode's maintained config is `rift-home-manager.nix`, which declares `xdg.configFile."opencode/opencode.json".text`. The installed `~/.config/opencode/opencode.json` is a symlink into the Home Manager generation. It currently defines a pinned Rift V2 plugin. Browser setup must extend this maintained declaration or factor the declaration into a common module. A runtime merge writer must not replace the symlink or try writing its Nix store target. The registry permits `opencode.jsonc` as an alternate file, but that is not the installed file here.

`voice-home-manager.nix` runs `scripts/install-voice-config.py` on activation. That script merges only `mcp_servers.dojo-desktop` into `~/.codex/config.toml`, respects `CODEX_HOME`, and backs up changed content. The project-local `.codex/config.toml` separately names `dojo-desktop`, launches `target/debug/dojo-agent --name t3code`, and allows six desktop tools. Global Codex config is an ordinary writable file. This task did not inspect its content.

`crates/dojo-agent-setup` already discovers installed agents, writes bundled skills, and merges MCP configuration using per-agent shapes. Its registry maps Codex to `~/.codex/config.toml` with `mcp_servers`, Claude Code to `~/.claude.json` with `mcpServers` through the Claude CLI, and OpenCode to its XDG config with `mcp`. Its shared skills directory is `~/.agents/skills`; Claude uses `~/.claude/skills`. Other agents also have registry entries. Pi explicitly has `mcp: None`. Do not invent a Pi MCP config file. The setup crate tracks owned files under `<Dojo home>/agent-setup.json` and avoids replacing user edits.

The voice assistant is a distinct Pi durable runtime. `tools/speech/pi-runtime.mjs` installs coding tools, human requests, the desktop extension, and optionally the CUA resource extension into its registry. Its instructions come from `memory-prompt.md` and `assistant.md`, not automatically from the ordinary Pi coding CLI's skills directory. `tools/speech/desktop-extension.mjs` contains a reusable stdio MCP client. Browser Control can use that client through a dedicated Pi extension, with its own cleanup and tool/session semantics. Alternatively, an ordinary coding Pi can use the pinned CLI through a shared skill. Merely installing a skill does not give the voice runtime Browser Control tools or workflow instructions.

The old browser path is agent tool -> Dojo desktop service -> CUA-derived Rust browser engine -> operator-approved or isolated CDP endpoint. The new upstream path is agent CLI/MCP -> detached local relay on 127.0.0.1:19989 -> extension -> existing Chromium profile. Browser Control has no model or planning loop. CLI and MCP share the relay but retain separate execute sessions. MCP initialization and tool discovery do not start the relay. The first operational tool does. Relay lifetime is independent of MCP process lifetime. Upstream restart is an explicit drain operation, not an ordinary startup side effect.

## Candidate package design

Use one reusable `nix/browser-control.nix` derivation in both devenv and Home Manager. Fetch the source with the exact Git commit and a source hash. A non-flake input in `devenv.yaml` would also pin the source, but a fixed `fetchFromGitHub` keeps the package self-contained and avoids another flake dependency. No released binary is required.

The actually pinned devenv nixpkgs is `cachix/devenv-nixpkgs` commit `c2f38fe7f9e04d9aadd354d380f2bd40531d9737`. Its own flake locks NixOS/nixpkgs at `c7def046b9a883d46974757852106483d741586f`, available locally at `/nix/store/akq5azpip8xzarlxqxis02lg2zz3hmcb-source`. This was established with offline flake metadata, rather than assuming another installed nixpkgs tree is current.

That source supplies `pkgs.pnpm_11` version 11.25.0, top-level `fetchPnpmDeps`, and `pnpmConfigHook`. The fetcher supports versions 3 and 4. It explicitly rejects version 3 with pnpm >=11 and requires `fetcherVersion = 4`. It fetches dependencies into a fixed-output store with scripts disabled, then the config hook installs offline with a frozen lockfile and scripts disabled. The pnpm 11 hooks set `pnpm_config_pm_on_fail=ignore`, preventing package-manager mismatch from downloading a requested version. If strict adherence to upstream pnpm 11.20.0 is desired, override the pinned `pnpm_11` package version/hash and use the same pnpm for both fetch and build. The package's generic derivation accepts version and hash and has the hook support. Its build scripts are ESM for version 11. Do not use `buildNpmPackage` with the upstream pnpm lock as though it were package-lock.json.

The natural derivation uses `stdenvNoCC.mkDerivation`, Node >=22.19, pnpm, `pnpmConfigHook`, `makeWrapper`, and a `pnpmDeps = fetchPnpmDeps { inherit src; pnpm = ...; fetcherVersion = 4; hash = ...; }`. Run upstream `pnpm build` after its offline dependency install. The workspace allows esbuild's install script; Nix hooks disable all scripts. esbuild platform dependencies may provide the binary directly, but the implementation must establish that or use nixpkgs esbuild with an explicit version-compatible binary override. Do not enable arbitrary install scripts merely to solve one build dependency.

`build-cli.ts` bundles local source but sets `packages: "external"`. Therefore the package must install runtime `node_modules` together with `package.json`, `dist`, `extension/dist`, `skills/browser-control`, and license/readme files. Shipping only dist is incomplete. Install under `$out/lib/browser-control` with wrappers for `browser-control` and `browser-control-mcp` executing the packaged Node and dist entrypoints. Keep modules adjacent to dist so ESM and detached relay subprocess resolution work. The extension path is then `$out/lib/browser-control/extension/dist`.

`build-cli.ts` embeds `new Date().toISOString()` as the build identity. Replace that expression with a deterministic identity tied to the pinned commit or source timestamp during the Nix build. Otherwise an identical source can produce different relay identities on rebuild. A pin alone does not fix this reproducibility defect.

Upstream builds type declarations through TypeScript in parallel with esbuild. Extension build bundles background, offscreen, and content scripts, then copies manifest, offscreen HTML, and icons. It does not require a running browser. The Rust `bc-studio` crate is outside the package build script and need not be built to install the CLI/MCP/extension.

For agent config, prefer the pinned manual MCP path and shared skill. Upstream's OpenCode plugin uses its managed cache and installs/registers a Browser skill and MCP together; its module calls V2 `ctx.mcp.transform` and `ctx.skill.transform`. Dojo's current OpenCode pin must be checked for that API before relying on it. A manual local MCP command avoids an independently updating npm cache and works with the declarative Home Manager config. Do not install both the plugin's workflow and the shared workflow into OpenCode without resolving duplicates.

## Extension installation and reversibility

Helium 0.19.2.1 is an AppImage wrapped by Home Manager. Its wrapper explicitly mounts `/etc/chromium/policies` read-only into its FHS environment. The actual existing profile directory is `~/.config/net.imput.helium`, with `Default` and `Local State` present. This task inspected directory existence only and did not read profile state. `~/.config/helium` does not exist here.

Upstream currently distributes an unpacked extension and documents Developer mode -> Load unpacked -> supplied directory. Its README expressly describes this as a user-approved installation. There is no published Chrome Web Store installation endpoint in the reviewed source. `ExtensionInstallForcelist` cannot install the unpacked folder merely by pointing at it. A force-install design would need a valid signed CRX and update manifest, or an actual Store entry; neither exists in the pinned distribution. Reusing managed policy infrastructure does not supply those missing artifacts.

Expose a stable generated user path such as `~/.local/share/dojo/browser-control/extension` through Home Manager pointing at the package's extension directory. Chromium stores the loaded directory, so the path must stay stable across Nix generations. The manifest includes a public key; its derived ID is `eibhgjafffkigblngnhafgbcipofaeon`, independent of the Nix store path. Keep this key intact. Source-key hashing calculated this ID; it has not been checked against a loaded extension.

A launcher with `--load-extension=<stable path>` is a possible automation route for Chromium-derived browsers, but support in this Helium build has not been established. Existing Chromium processes normally reuse a profile process, so new launch flags cannot be assumed to change an already running browser. Do not kill the existing profile browser to activate a flag. A separate temporary profile can investigate support without touching the existing profile, after which installation can use the supported path. Direct edits to Chromium Preferences/Secure Preferences while running are neither a sound activation method nor a maintained source.

If policy installation becomes warranted, modify maintained system-config sources and use `scripts/install-system-config.sh`. Its Chromium directories are root:root 0755. Existing privileged helpers and sudoers files are copied into place, not symlinked into user-writable checkout. The theme helper is intentionally restricted to browser color and cannot become a general extension policy writer. Keep extension policy in its own managed JSON file and preserve BrowserThemeColor. Chromium reads backup files inside the managed directory; backups must go in a sibling backup directory. Existing policy directory creation and Helium mounting can be reused without broadening the theme sudoers privilege.

Reversibility comes from Home Manager generation rollback, backup-before-merge for ordinary agent config, a stable symlink to the immutable extension, and explicit extension disable/removal in the browser. Do not delete profile data, browser tabs, Browser Control relay/session files, or secret profiles during a package rollback. An already-running relay retains old code; upstream ordinary commands refuse stale relay replacement and require explicit `browser-control relay restart` after active work drains. Extension reload is likewise an activation step after package updates.

## Exact cleanup inventory and retained dependencies

The old browser implementation is concentrated in three files to remove after callers migrate:

- `src/bin/dojo-agent/browser.rs`
- `src/bin/dojo-agent/browser_mutation.rs`
- `src/bin/dojo-agent/browser_owned.rs`

Update `src/bin/dojo-agent.rs` to delete those module declarations, Browser request structs, five `desktop_browser_*` tools, browser-specific capability claims, owned-browser supervisor command handling, and corresponding browser tests. Update `src/bin/dojo-agent/service.rs` to remove BrowserCreate/Close/Observe/Navigate/Inspect message variants, request helpers, browser session/owned-browser/expiry fields, browser teardown and invalidation logic, and corresponding dispatch branches. Its verification path currently consumes the old exact-bound DOM observations, so remove or replace that source rather than leaving dead verification claims.

Update `tools/speech/desktop-extension.mjs` to remove old browser names from replay/cleanup lists and remove `desktop_browser_navigate` external-unknown bookkeeping when the tool no longer exists. `assistant.md` currently instructs operator endpoints and default-disabled browser navigation; rewrite that paragraph around Browser Control's supported workflow. Update `docs/agent-input.md` CUA-derived capabilities and policy examples, bundled `dojo-desktop` skill wherever it tells agents to use retired browser tools, `.scratch/cua-adoption/spec.md` and relevant browser tickets, and `docs/research/browser-control-replacement.md` as necessary. Replace or retire `tests/browser_owned_desktop.rs` and browser-specific tests in `tests/desktop_policy.rs`. The live browser-window-launch check in `scripts/check-voice-browser.py` tests ordinary desktop launch, not old CDP, and is still relevant.

Do not delete the full `dojo-cua-browser` Cargo dependency just because the browser adapter is removed. Despite its alias, it is the whole CUA driver core package. `src/bin/dojo-agent/policy.rs` consumes `dojo_cua_browser::host_policy::HostPolicy`, and `src/application_catalogue.rs` consumes `dojo_cua_browser::unix_secs_to_rfc3339`. These independent callers must move to a retained appropriate crate/helper before the Cargo dependency can be deleted. The host policy tests still matter.

`tools/speech/cua-extension.mjs` is a CUA resource bridge selected from `vendor/cua/metadata/pi-resource-catalog.json`; it is not the old Dojo browser adapter. Do not delete it solely due to a browser migration. Likewise native AT-SPI, desktop input/capture, computer model handling, and CUA perception retain separate ownership.

The compositor final-writer path in `crates/dojo-external-admission` and `vendor/niri/src/external_admission*` carries browser navigation semantics. Search its callers after deleting `browser_mutation.rs` before deleting this optional route. It cannot remain advertised as Browser Control protection because Browser Control executes through its extension and bypasses that Niri final-writer mechanism. Any retained shared admission functionality needs an explicit caller and purpose.

`crates/dojo-computer-verification` has generic BrowserDom evidence and exact-bound predicates. Those types are not automatically dead when one adapter is removed, but a Browser Control tab result cannot be called exact native-window verification without a proven mapping. Reassess the retained predicates and capabilities against real new observations.

## Material integration risks

The new execution authority differs from the old Dojo navigation admission. Browser Control intentionally operates the existing profile after extension approval and can run arbitrary Playwright. Its action results must not be described as Niri capture-authorized mutations or exact native-window receipts. A Dojo native desktop refusal must not silently become authorization to retry the same action through browser tools.

OpenCode's config is declarative and shared with Rift; agent config activation must preserve that ownership. Agent setup state currently records one MCP ownership entry per file, so generalizing it for an additional browser server requires inspecting that representation, not merely calling `configure_mcp` twice. Pi coding CLI skill support does not configure Pi durable voice. Both installed clients and the project-local T3/Codex configuration need a concrete setup decision.

No live extension install, Helium flag compatibility, build dependency hash, npm package fetch, or browser operation was verified in this grounding task.
