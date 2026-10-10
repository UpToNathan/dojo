# Define and migrate the complete shared role contract

Status: needs-triage

## Scope

Inventory all shared, syntax, and editor-specific roles read by exporters and the shell. Resolve the spec's proposed structs against that inventory. Keep static presets and wallpaper generation as peer producers.

## Acceptance

- Encode required roles and color pairs in types; replace magenta sentinels for resolved output.
- Distinguish hover, selection, focus, disabled text, accent fills, and status backgrounds.
- Fix T3's warning surface mapping to a success diff background.
- Preserve authored editor-specific roles and exact old-generation values.
- Centralize required-role validation and migrate internal readers together.
- Test representative actual exported role values, not only role counts.

## Unresolved

The exact role inventory and old-generation translation must be specified before this is ready for implementation.
