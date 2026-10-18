# 06 — Export opencode's optional `text.faint` theme token

Type: task
Status: ready-for-agent
Blocked by: none
Effort: small

## Brief

opencode `c5e513e98` (in the pinned `501b15af`) adds an optional `text.faint` token to its V2 theme
schema, defaulting to `text.muted`. Dojo's exporter in `src/theme/export.rs` does not emit it.
Emit it from `foreground_muted` (dimmer than `text.muted` only if the existing role map has a
suitable colour; otherwise leave the default), keeping existing themes valid, and add a test beside
the other opencode export tests. Update `docs/theme-application.md`'s opencode row.

## Acceptance

Exported opencode themes include the token when the palette supports it; the export test and the
opencode schema validation in the existing audit pass.
