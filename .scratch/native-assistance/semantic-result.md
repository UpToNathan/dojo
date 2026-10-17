# Semantic accessibility delivery

## Implemented and verified

Semantic invoke, text replacement, numeric value changes and direct child selection now use fresh session-owned references and a compositor-owned final send. Human takeover before dispatch refuses the mutation. Unconfirmed effects remain unknown and block follow-on mutations in the same target process until a matching terminal reply arrives.

Exact native surface bindings are implemented for GTK and the generic annotation protocol. Qt5, Qt6 and repository GPUI producers publish their actual accessibility bus and root. There is no application allowlist. Unmodified Electron, XWayland and other runtimes without a producer remain unsupported for exact semantic actions. They retain the existing visual/native input route. Helium web content remains on browser-control.

## Evidence

All 17 live checks passed using actual applications, AT-SPI, private D-Bus and nested Niri on the inactive test workspace. The active eDP-1 workspace remained unchanged. GTK 4.22.4 exercised all four actions; Qt 5.15.19 and Qt 6.11.2 exercised text replacement; repository GPUI exercised invocation. Each runtime verified sibling-window separation. References were checked for foreign ownership, replay, expiry, closed targets and stale observations; human takeover produced no effect.

- Full live evidence: `/tmp/dojo-semantic-live.Qqjyg6/acceptance.json`.
- Compact acceptance: `/tmp/dojo-semantic-acceptance-summary.json`.
- Niri SHA256: `6f4043e227c325bc8ddda4f2a42069bb15ea5b557eefc4be05bf0fc9b807b1cb`.
- Agent SHA256: `2272b495a9d9e7c449310621ccb804c3d8dacc61922a02893f33c2f060e3f33f`.
- Immutable agent: `/nix/store/k5lg480pk74jjf5vlyrnk1bs5crdb31x-dojo-agent`.

Focused suites passed: accessibility 18; dojo-agent 41; external admission/transport 28 with one ignored; browser wire helpers 14; Pi transport/trace 39; generic protocol 5; process-wide unknown fence 1. Launcher, generator and installer checks passed. The original CUA Driver was rebuilt against the changed IPC.

## Deployment

Home Manager generation `/nix/store/6wmd6gnfy2v6y6jcknhypihcz02gwjqx-home-manager-generation` was activated. Qt producers and stable agent launcher are installed, toolkit accessibility is enabled, and voice/MCP configuration uses the trusted immutable agent. Niri is staged at `~/.local/share/dojo/niri/ed4573e557c6b000`. The next Niri UWSM login activates it. The running compositor was not restarted.

New adopted sources are pinned in `provenance/native-accessibility.json`, toolkit provenance files and the CUA ledger. The repository-wide reference inventory still contains legacy unresolved references; this delivery does not claim every historical reference is resolved.

## Test isolation incident

An early private-bus attempt allowed standard session service activation and disrupted darkman's socket. The service was restored and `darkman get` returned dark. The final runner uses a private runtime, memory settings and a bus service whitelist containing only the accessibility launcher. Final acceptance used that corrected isolation.

See [native accessibility](../../docs/native-accessibility.md) for API behavior and capability limits.

## Full-scope completion audit

The delivery above is complete for its four tested runtime families plus generic
annotation producers. This section records an Electron investigation that was
later abandoned; see the scope decision at the end. Issue 02 is closed on that
scope.
T3's currently installed runtime is Electron 44.4.2 with Chromium
152.0.7977.130. The separately installed Electron used by Legcord is 43.6.0
with Chromium 150.0.7871.250. These are distinct integration targets.

`provenance/native-electron-investigation.json` records the observed T3 binary
hash and its exact Electron/Chromium source commits. Recheck the actual binary
before applying any runtime patch:

```sh
node tools/accessibility/probe-electron-runtime.mjs \
  --pins provenance/native-electron-investigation.json \
  ~/.local/opt/t3code/t3code
```

The real pin passed; a deliberately changed hash was rejected. The probe starts
Electron in Node mode and opens no window or browser profile. It proves runtime
identity, not accessibility binding. Electron's native producer is still under
investigation and is not installed or verified.

## Electron producer progress

A pinned Chromium 152 producer patch now connects each Linux Views host's own
root ATK object through a typed PlatformWindow setter to that same host's
Wayland root surface. Its source preimages and patch hashes are recorded in
`tools/accessibility/electron/chromium-producer-provenance.json`. Patch application
after Electron's overlapping teardown fix passed, and the canonical protocol's
generated C compiled. Independent source review found no concrete blocker. The
full C++ producer has not compiled or run yet.

The new pinned AT-SPI bridge accessor was built and exercised against an actual
private bus, launcher and registry. It returned the exact two objects' names
through their exported AT-SPI addresses, refused uninitialized, defunct and
foreign-thread access, refused after cleanup, retired its old bus owner, then
reinitialized and served both objects under its new owner. The fixture exposed
and reproduced an upstream stale registry-ID bug after reinitialization; the
bridge patch fixes it. Evidence is `/tmp/dojo-atk-endpoint-acceptance.json`.
No host bridge, Electron runtime or running T3 instance was replaced.

The Electron source workspace is prepared from exact pinned revisions.
`tools/accessibility/electron/prepare-source.mjs --sync` is fetching the
Chromium/dependency tree with depot_tools automatic updates disabled. Source
synchronization, full compilation and isolated two-window lifecycle/action
acceptance remain outstanding.

The optional Electron fixture now includes text, invocation, numeric and
selection controls plus hide/show and recreation commands. Its native branch
remains unexecuted until the custom runtime is built. The extended live runner
was regression-tested against GTK, Qt5, Qt6 and GPUI with the final installed Qt
packages; all checks passed at
`/tmp/dojo-semantic-live.MpKHux/acceptance.json`. That run did not enable Electron
and does not add Electron coverage.

The six modified Chromium integration objects compiled successfully against the
complete pinned checkout and applied Electron patch series. The compiled Views
object contains `RefreshAccessibilityAnnotation`; this was not a build with the
ATK producer compiled out. Object hashes and source commit are in
`/tmp/dojo-electron-producer-object-acceptance.json`. Full runtime distribution
compilation is now running, with no live Electron result claimed.

The runtime packaging recipe reuses the pinned Nixpkgs binary recipe. Its
original bytes, MIT license and reproducing adaptation patch are recorded in
`tools/accessibility/electron/nix-packaging-provenance.json`. Nix parsing and
patch reproduction passed. Packaging and launch still await the actual compiled
archive; these source checks do not prove a usable runtime package.

## Scope decision

The Electron producer work above is abandoned by decision. No Chromium build is run or maintained; Chromium apps are supported through OCR/vision and native input only. The completion audit's open Electron objective is closed as won't-do. Delivered coverage is the four tested runtime families plus generic annotation producers.
