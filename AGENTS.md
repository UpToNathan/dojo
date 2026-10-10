## Working in Dojo

Dojo has two kinds of tracked work: a Rust Niri desktop shell and the desktop configuration that runs around it. Check the relevant source and docs before changing either.

### Specs and terminology

- For feature requests or implementation tickets, read the matching `.scratch/<feature-slug>/spec.md` and issue file. Follow the local issue format in [docs/agents/issue-tracker.md](docs/agents/issue-tracker.md).
- When a ticket needs triage, use the labels described in [docs/agents/triage-labels.md](docs/agents/triage-labels.md).
- Before exploring a domain or proposing a design, read [GLOSSARY.md](GLOSSARY.md) and the relevant decisions in `docs/adr/`. Follow [docs/agents/domain.md](docs/agents/domain.md) for vocabulary and ADR conflicts.

### Desktop configuration

The repository also maintains devenv inputs, Home Manager modules, application configs, and system configuration. Before changing installed system files, read [system-config/README.md](system-config/README.md) to distinguish tracked inputs from generated runtime state. Use `scripts/install-system-config.sh` to install those files; it preserves replaced files as timestamped backups.

### Rust changes

Read the relevant module and its callers before changing behavior. Keep configuration parsing and external input validation at the boundary, and keep domain logic independent of GPUI, Niri, and host services where the existing design allows it.

Use the build and test commands documented in [README.md](README.md), from `devenv`. Add or update tests for changed behavior, especially for pure logic and parsing. Run the checks that cover the changed code before reporting completion.

### Source of truth

Prefer existing config files, scripts, and docs over copying their details into this file. Keep generated caches and machine-local runtime state out of tracked source changes.
