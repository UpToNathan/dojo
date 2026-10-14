# dojo-agent-setup

This crate adapts the MIT `cua-agent-setup` package at CUA commit
`88aff7e5424e9ed45b93ded85eb4111472ca5190`. It retains the agent registry,
read-only detection, JSONC/TOML/YAML config editing, timestamped backups,
atomic writes, per-user state, and reversible remove/update behavior. The
adaptation replaces CUA server/skill definitions with `dojo-agent` and the
bundled `dojo-desktop` skill. See `PROVENANCE.md` and `CUA-MIT-LICENSE.txt`.

`dojo-agent-setup` defaults to a read-only plan. It changes user configuration
only with `--apply`; use `--root <temporary-home>` for hermetic planning and
integration tests. The library's `HostEnv::isolated` supports the same sandbox
boundary. This crate does not alter credentials, global providers, shell
configuration, or system files.

The bundled skill is intentionally limited to Dojo's current advertised
capabilities. It does not claim parity with CUA's browser, accessibility, or
text-entry tools.
