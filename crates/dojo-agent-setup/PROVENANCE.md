# Import provenance

- Upstream: `trycua/cua`, commit `88aff7e5424e9ed45b93ded85eb4111472ca5190`.
- Source: `libs/cua/crates/cua-agent-setup/src/{edit,fsutil,registry,state,skills,lib}.rs` and `tests/agents.rs`. The upstream `config.rs` module was excluded because it manages CUA provider preferences, which are outside Dojo agent onboarding.
- License: MIT; notice retained in `CUA-MIT-LICENSE.txt`.
- Status: source imported and adapted, not byte-identical. `edit.rs` retains the format-preserving JSONC/TOML/YAML editor and its tests. `fsutil.rs` retains backup, atomic write, symlink, mode and hash behavior. Registry/detection and reversible state/skill logic are retained. Dojo-specific edits rename state/environment/server identity, embed only `dojo-desktop`, add a plan operation, and replace CUA command discovery with `dojo-agent`. The CLI defaults to plan and requires `--apply` for writes. CUA-specific server names, bundled skills, legacy cleanup, and CUA CLI commands are excluded.
- Dependency requirements use compatible semver ranges; the standalone `Cargo.lock` records tested resolutions. No CUA agent runtime or provider dependency is imported.
- Tests retain upstream format-edit tests and adapted agent-config cases, translated to Dojo server identity, plus sandbox-only dry-run/apply checks.
