# Select the published Dojo theme in T3 nightly

Status: resolved

## Problem

`t3 theme show` reported `Environment theme: not set` and `Published themes: dojo`. Dojo's T3 target publishes correctly but never selects its theme.

## Acceptance

- After regular-file publication, use the desktop's installed `~/.t3/bin/t3 theme set dojo --base-dir ROOT` command.
- Pass the same root used for publication, including a configured `T3CODE_HOME`.
- Report `reload_requested` when selection succeeds, `reload_failed` when selection fails after publication, and `needs_selection` when the bundled CLI is absent.
- Retain ownership refusal for unowned or edited outputs and preserve unrelated T3 settings.
- Test successful selection, failed selection, and missing CLI with an isolated home. Verify the installed nightly accepts and selects the generated file.

## Comments

Implemented during this design pass in `src/theme/apply.rs`. Installed CLI selection now reports `dojo`. All 27 library tests pass, including isolated successful, failed, and missing-CLI selection. The built `dojo-theme` production Apply path also returns `reload_requested` with the installed nightly, using a temporary Dojo store while preserving the original desktop selection and target list. This verifies selection and publication, not every rendered app color.
