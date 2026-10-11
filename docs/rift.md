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
