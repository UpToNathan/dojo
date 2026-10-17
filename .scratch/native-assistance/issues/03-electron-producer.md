# Exact Electron native accessibility producer

Type: task
Status: wontfix

## Brief

Extend exact native accessibility binding and semantic actions to the installed
Electron runtime family. The current T3 source target is Electron 44.4.2 and
Chromium 152.0.7977.130, with exact commits and binary identity recorded in
`provenance/native-electron-investigation.json`. This is part of issue 02's
original broad non-GTK objective.

## Acceptance

- Build the pinned Chromium producer patch and matching bridge accessor.
- Publish each Linux Views host's own ATK root on its own Wayland surface.
- Revoke stale bindings across hide, teardown, accessibility disable and bridge
  loss; publish current bindings when those capabilities return.
- Exercise all supported semantic actions against actual independent controls.
- Prove same-process sibling isolation with two identically titled windows.
- Reject old references after native window retirement and root recreation.
- Use the private accessibility bus, disposable profile and inactive nested test
  workspace. Do not touch T3's real profile or replace its running runtime.
- Pin added build inputs and dependency revisions. Keep browser-control as the
  Helium interface.
- Deploy only after the actual native runtime acceptance passes.

## Current evidence

The bridge accessor and stale registry-ID fix pass the headless private-bus
fixture. The producer source patch applies to the actual pinned Chromium
checkout, and independent source review found no concrete blocker.
The full dependency sync is running. A disposable Electron fixture and optional
live-runner branch now cover four actions, sibling isolation, hide/show and root
recreation; they have passed syntax checks but have not run against a compiled
producer. See [source grounding](../electron-ground.md).

## Build progress

The full source/dependency sync and Electron's upstream hooks completed. All
resolved inputs are recorded in the source revision lock. The producer is the
last commit in the applied Chromium patch series. GN generation passed with
local execution and the pinned checkout's tools. The six changed integration
objects are compiling with four lower-priority jobs; no object-build or full
runtime success is claimed yet.

The native N-API lifecycle fixture addon built and loaded successfully. It will
let the isolated Electron fixture exercise bridge cleanup/reinitialization in
its own process, withdraw both window bindings, refuse stale references and
verify restored controls. Its Electron runtime checks remain pending.

The six changed native integration objects compiled successfully. Object hashes
and the actual patched source commit are recorded at
`/tmp/dojo-electron-producer-object-acceptance.json`. The full Electron
distribution build is now running. No live Electron coverage is claimed yet.

The fixture and optional runner now also require accessibility disable/re-enable
to withdraw both annotations, refuse a formerly valid text action and restore
distinct roots with the previous text value. Both edited scripts pass syntax
checks. This branch still awaits the actual compiled runtime; it is not passing
coverage evidence.

Nix package evaluation now passes beyond parsing: the evaluated runtime
derivation contains the exact patched bridge path, and an incorrect archive
hash is rejected. Evaluation used an empty placeholder archive and did not
build or install it. This proves recipe dependency/argument wiring only.
The actual runtime package remains pending. Evidence:
`/tmp/dojo-electron-package-evaluation.json`.

## Decision (2026-10-15)

Won't do. Building and maintaining a patched Chromium/Electron runtime is out of
scope. The full runtime compile was started (35,572 steps) and stopped by the
user's decision; no runtime was packaged, installed or verified. Chromium apps
handle their own accessibility; Dojo supports them via OCR/vision (existing
perception worker) and native input. The producer patch, bridge accessor and
fixture are retained as unverified reference material only.
