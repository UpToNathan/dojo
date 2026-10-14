# Browser Control migration result

Implementation and configuration installed. After login, the native browser
socket and pinned broker/MCP environment are active. Helium was not running at
the post-login check; its installed launcher loads the extension when opened.

## Source and package

- Upstream `anomalyco/browser-control` `0.8.4`, commit `b4e4238742fa73daabfb664f1108113e9577849d`.
- Literal copy: `vendor/browser-control/upstream`; 227 source files, 24 local changes.
- Reproducible patch: `vendor/browser-control/dojo.patch`.
- Provenance: `vendor/browser-control/manifest.json`, identity `33ce082f4b3b75f6f8ddaf321d8ea904394f6004015eb3f4176ad445a396055b`.
- Package: `/nix/store/wgfy4x2ybm5pjvg6wwmrqddlh8flw9b0-dojo-browser-control-0.8.4-dojo`.
- Broker: `/nix/store/1d05mgisc0lfgnh33d6154d8jkczi5pq-dojo-browser-control-broker-0.1.0`.
- Nix pins pnpm 11.20.0, frozen dependencies, compiler/broker lock, patched build identity and compositor source hashes.

## Changes

Browser Control now uses Niri's final socket writer with explicit generation
binding, browser-client takeover and retirement. Commands larger than 1 MiB
refuse before transmission; verified socket buffers support normal Playwright
injected JavaScript. Partial writes and uncertain Chrome effects never replay.

Each resumed session generation receives a fresh sandbox while preserving its
registered target and tab ownership. Delayed old preparation, callbacks and CDP
connections retain the old generation. Cancellation runs outside the execution
queue. Shutdown retires private sessions. Browser focus activation is refused.

Pi owns its browser session and task lifecycle, records uncertain outcomes,
blocks automatic browser continuation and desktop input fallback, and cancels
one RPC after native retirement without killing unrelated requests. Canonical
history remains in Pi; duplicate execute journals are disabled.

The former CUA desktop_browser routes and four Dojo adapters/tests were removed.
Policy, perception, timestamps and native desktop Driver remain. Historical
adapter hashes are preserved in `vendor/cua/provenance/retired-browser-adapters.json`.

## Verification

- Fork: type/local checks, Dojo CLI/extension build, 1,005 tests across 72 files.
- Native broker: 15 tests, including actual 400 KB Unix/TCP transport, over-limit refusal, partial writes and disconnect.
- Niri: four takeover fixtures plus operator handoff CLI validation.
- Root Rust agent: 35 tests; native policy/catalog test passed.
- Pi/speech: 110 tests passed, one optional CUA SDK live test skipped.
- Config merger: three fixtures; immutable Niri installer fixture passed.
- Exact package and Home Manager generation built; Home Manager deployed to venus.
- CUA ledger: 514 verified files, 1,716 whole-import files and 29 reproduced patches.

Real Helium acceptance: `/tmp/dojo-browser-live.SgAaEh/acceptance.json`.
The isolated browser filled a textbox, clicked a button, read the actual result,
retired a generation, resumed and preserved its page, then cancelled delayed
JavaScript and verified that no late effect appeared in the next generation.
Both monitors' active workspaces remained unchanged; dojo-tests stayed inactive.
The test used a temporary profile and the release nested compositor. No Chrome
or Chromium browser was installed, and the authenticated Helium profile was not used.

## Activation and limits

Home Manager installs the stable extension path, immutable MCP/broker commands,
shared skill, maintained OpenCode config and preserving Codex/Claude/Zed merges.
The Niri installer retains the previous generation and roots both executable pins.
A logout/login and normal Helium relaunch are needed to activate the new native
channel and extension. They close current apps and require the operator's go-ahead.

Native admission is a kernel write, not proof that a browser effect completed.
Already admitted effects and page timers may continue after cancellation. An
uncertain in-flight outcome remains blocked and requires operator review; native
transport uncertainty requires reconnection. Tab-to-native-window mapping is
conservative browser-client scope. No durable approval database was added.

## Post-login check

The running Niri installation is `786882e3dec1ae37` with the accepted broker and
MCP pins, and `/run/user/1000/dojo-browser-admission.sock` exists with mode 0600.
Voice is managed by systemd and reports `speech worker ready`.

A later Niri edit exposed an incorrect packaging dependency: browser package
evaluation checked live Niri source hashes even though it does not build Niri.
Removed that package assertion. Fork, patch and broker checks remain mandatory;
the historical compositor snapshot remains available to the explicit provenance
audit. Devenv evaluation now succeeds without silently adopting unrelated changes.
