## Working in Dojo

Dojo has two kinds of tracked work: a Rust Niri desktop shell and the desktop configuration that runs around it. Check the relevant source and docs before changing either.

### Specs and terminology

- For feature requests or implementation tickets, read the matching `.scratch/<feature-slug>/spec.md` and issue file. Follow the local issue format in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).
- When a ticket needs triage, use the labels described in [docs/agents/triage-labels.md](docs/agents/triage-labels.md).
- Before exploring a domain or proposing a design, read [GLOSSARY.md](GLOSSARY.md) and the relevant decisions in `docs/adr/`. Follow [docs/agents/domain.md](docs/agents/domain.md) for vocabulary and ADR conflicts.

### Desktop configuration

The repository also maintains devenv inputs, Home Manager modules, application configs, and system configuration. Before changing installed system files, read [system-config/README.md](system-config/README.md) to distinguish tracked inputs from generated runtime state. Use `scripts/install-system-config.sh` to install those files; it preserves replaced files as timestamped backups.

### Isolated workspaces and build disk

Use Rift for isolated or parallel work. Do not use git worktrees: no `git worktree add`, no `EnterWorktree`, no `isolation: "worktree"` on subagents, and no worktree workspace strategy when launching threads. A worktree starts with empty build directories and gets its own multi-gigabyte `target/`. A Rift workspace is a copy-on-write snapshot of this checkout, created in seconds, and builds warm through the shared mbx cache. Details are in [docs/rift.md](docs/rift.md).

```sh
rift create --name <task>        # from the main checkout; prints ~/.rifts/Dojo/<task>
cd ~/.rifts/Dojo/<task>          # HEAD is detached: git switch -c <branch> before committing
rift remove                      # from inside the workspace when finished; then `rift gc`
```

- Work only inside your workspace and leave the main checkout alone. Rift documents no merge-back; the main checkout integrates with `git fetch ~/.rifts/Dojo/<task> <branch>` followed by a merge or cherry-pick.
- Remove the workspace once its branch is integrated or abandoned. Never `rift init` a busy checkout; the main checkout is already registered.
- Run Cargo through `devenv shell --`. That puts the mbx shim first on `PATH`, which shares compiled work across checkouts and prunes its own storage. Do not point `CARGO_TARGET_DIR` at a private directory, and do not leave build outputs in `/tmp`.
- Do not leave `tmp_pack_*` files, spike targets or other large scratch output behind. Check `du -sh` on anything you created before you finish.

### Runtime never uses devenv

devenv is the development environment: use it to build, test and install. Nothing Dojo runs on the desktop (systemd units, keybinding commands, wrappers, the voice service and its workers, the shell and its helpers) may start through `devenv shell`, evaluate devenv, or depend on its `PATH` or variables. Installed binaries get their libraries from the installers' `environment.sh` files and GC-rooted store paths; add anything else a runtime process needs (as `install-helpers.sh` does for `speech-environment.sh`) rather than borrowing devenv's environment.

### Rust changes

Read the relevant module and its callers before changing behavior. Keep configuration parsing and external input validation at the boundary, and keep domain logic independent of GPUI, Niri, and host services where the existing design allows it.

Use the build and test commands documented in [README.md](README.md), from `devenv`. Add or update tests for changed behavior, especially for pure logic and parsing. Run the checks that cover the changed code before reporting completion.

### Source of truth

Prefer existing config files, scripts, and docs over copying their details into this file. Keep generated caches and machine-local runtime state out of tracked source changes.
