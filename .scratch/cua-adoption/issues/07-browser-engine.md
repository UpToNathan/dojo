# 07 — Full CUA browser engine with a Niri BrowserPlatform adapter

Type: task
Status: superseded
Blocked by: 01, 02, 03, 04
Mutation enablement blocked by: 16
Effort: large; isolated profile and existing profile are separate milestones

## Brief

Import Driver core `browser/` coherently: transport, binding, refs, semantic snapshots, mutation, frames/OOPIFs, dialogs, downloads, preparation, grants and cleanup. Build Dojo `BrowserPlatform` identity/geometry/process/endpoint adapter. Prove PID/start-time/executable/process-tree/window binding and loopback endpoint ownership. Handle trusted Nix store symlinks/wrappers explicitly; upstream canonical-path rejection is not a reason to allow arbitrary redirects.

Start with driver-owned isolated Chromium. Later add explicit consented existing-profile attachment. Observation must not enable debugging, steal/relaunch profiles or modify authenticated user state. Existing-profile foreground requirements that violate human-focus isolation are refused until a permitted route exists. CDP mutations require ticket 16's proved shared admission; until then browser capabilities are read-only/refused for mutation. An ownership query plus independent network dispatch is not adequate; no hidden side-channel bypass.

## Acceptance

- Keep upstream CDP tests; real local browser pages exercise navigation, refs, frames, dialogs and downloads.
- Wrong endpoint/window/PID reuse, stale refs, absent geometry and missing grants fail closed.
- Browser launch/profile creation is unsafe to replay; crash cleanup reconciles only owned processes/profiles.
- Download paths/symlinks and file authority validated; sensitive browsing content excluded from default recording.
- Dispose session reaps only Dojo-owned processes; existing user browser remains intact.

## Comments


Superseded by [the pinned Browser Control migration](../../browser-control/spec.md).
Dojo uses Helium; no additional Chrome or Chromium installation is authorized.
