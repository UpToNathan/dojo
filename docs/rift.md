# Rift workspaces

Dojo pins the Rift CLI to v0.0.13 from its upstream Linux x86_64 release and
configures the matching OpenCode V2 plugin. The CLI is available in both
Home Manager and the devenv shell. OpenCode resolves the plugin source from the
same tagged Rift repository release. The plugin adds `rift.create`, `rift.list`,
and `rift.remove` agent tools and integrates Rift with OpenCode V2's workspace
UI and session-move tool; it does not configure other agent clients. A project
must be registered with `rift init` before these project-specific workspace
features are available. Home Manager installs/configures the plugin but does not
register any project.

In a Git workspace, initialize Rift once, then create and manage workspaces:

```sh
cd ~/code/project
rift init
rift create --name parser-fix
rift list
rift remove ~/code/.rifts/project/parser-fix
```

`rift init` registers the selected directory. By default, it selects the
nearest registered Rift root or, if none exists, the nearest Git root. On
Linux, Rift v0.0.13 supports btrfs subvolume snapshots and filesystems with
working per-file reflinks (such as XFS). On btrfs, initializing an ordinary
directory creates a sibling staging subvolume, reflink-imports every entry,
moves the original directory aside, activates the subvolume at the original
path, then recursively removes the original. This import is not filtered, so
generated build and dependency directories are copied too. Do not initialize a
busy checkout: open shells and applications can still refer to the old
directory while Rift replaces its path. Rift has no full-copy fallback when the
filesystem cannot provide the required copy-on-write operation, so confirm
filesystem support before initializing a workspace. With the configured
`hooks = true`, `.rift.toml` lifecycle commands run during workspace creation
and removal. Review Rift's
[upstream README](https://github.com/anomalyco/rift/tree/v0.0.13#readme) and
[OpenCode plugin README](https://github.com/anomalyco/rift/tree/v0.0.13/plugins/opencode#readme)
for storage behavior, hook semantics, and command details. The pinned
[btrfs initializer source](https://github.com/anomalyco/rift/blob/v0.0.13/crates/core/src/strategy/btrfs.rs)
shows the exact directory replacement sequence.

Rift is experimental. Dojo only installs and configures it; it does not run
`rift init` or create workspaces automatically.

## Working in Dojo

The main Dojo checkout is a btrfs subvolume and is already registered, so agents
only run `rift create`. Agents use Rift instead of git worktrees; see
[AGENTS.md](../AGENTS.md#isolated-workspaces-and-build-disk).

- `rift create --name <task>` from the main checkout took about 4 seconds
  here and produced `~/.rifts/Dojo/<task>`. The copy carries the working tree,
  including uncommitted changes, with `HEAD` detached at the source commit.
- The default copy omits `target/` and other regenerable directories. The mbx
  cargo shim, put first on `PATH` by `devenv shell`, restores compiled work from
  its shared cache. A first `cargo check` of a small crate in a fresh workspace
  was served from that cache (69 compilations, about 1m20s saved).
- Rift documents no merge-back. Commit on a branch in the workspace
  (`git switch -c <branch>`), then integrate from the main checkout with
  `git fetch ~/.rifts/Dojo/<task> <branch>` and a merge or cherry-pick. Fetching
  from the workspace path was verified.
- `rift remove` run inside a workspace moves it to `~/.rifts/Dojo/.trash`;
  `rift gc` deletes trashed workspaces. Disk is only reclaimed after `rift gc`.
- `rift create` refuses repositories with an in-progress merge, rebase,
  cherry-pick, revert, bisect or lock files. Stale entries from old worktrees
  are removed with `git worktree prune`.

## Build disk

- `mbx` is the build cache. It keeps compiled work in `~/.cache/mbx` under a
  45 GiB budget and prunes itself. Check it with `mbx stats` and `mbx doctor`.
- `scripts/prune-installations.sh ROOT [KEEP] [--apply]` removes old revisions
  under `~/.local/share/dojo/desktop` and `~/.local/share/dojo/niri`. The
  installers run it and keep the four newest, `current`, `previous` and any
  revision a running process executes from. Without `--apply` it only lists.
  Run `nix-collect-garbage` afterwards to reclaim the store paths those
  revisions rooted.
