# Replace the CUA browser adapter with pinned Browser Control

Type: task
Status: claimed

## Goal

Implement the [migration spec](../spec.md), install one pinned package through
Dojo's devenv and Home Manager, and configure Pi and installed MCP clients.

## Acceptance

- Literal upstream copy and reproducible patch with immutable source/dependency pins.
- Native generation binding, final socket writer, human takeover, cancellation and uncertainty handling.
- Preserve adopted and created tabs; no focus stealing, raw CDP or CLI bypass.
- Remove superseded desktop_browser tools and migrate callers.
- Build exact package and Home Manager generation; prove real browser behavior in isolation.
- Install configuration; request disruptive desktop/browser restart only after checks.

## Comments

Source frozen at upstream `b4e4238742fa73daabfb664f1108113e9577849d`.
The fork passed 996 upstream/fork tests. Native broker passed 13 tests and
Niri passed four takeover fixtures. Pi/desktop runtime checks are recorded in
[the result](../result.md) as they complete.
