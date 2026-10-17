# Pinned Electron Wayland accessibility producer

This directory contains a source patch for Electron 44.4.2 and its pinned
Chromium 152.0.7977.130. It has not yet passed a full Electron build or runtime
fixture. It does not change the installed T3 runtime.

Apply `chromium-wayland-atspi-annotation.patch` after Electron's Chromium patch
series. The patch requires the separately patched at-spi2-core bridge export
`dojo_atk_bridge_dup_accessible_reference`. Without that export, the producer
publishes nothing. Use an isolated runtime library closure for that dependency.

Before applying the patch, check the exact source inputs and patch hash:

```sh
node tools/accessibility/electron/check-producer-patch.mjs /path/to/chromium/src
```

The checker reads source files and runs `git apply --check`; it does not modify
the checkout. It accepts Electron's pinned overlapping Wayland teardown fix.
Other changes to producer input files require explicit review and new pins.

The producer joins each Views widget's own root ATK object to its own native
Wayland root surface. It uses Chromium's UI thread, existing registry dispatch,
and existing compositor frame pipeline. A 250 ms timer reconciles accessibility
mode, root, and bridge identity. Hide and destruction revoke immediately.

`chromium-producer-provenance.json` records upstream revisions, exact input and
patch hashes, copied code origins, and validation status. `CHROMIUM-LICENSE` is
the literal license from that Chromium revision. The protocol retains its own
copyright and license in the patch. Detailed source evidence and remaining live
checks are in `.scratch/native-assistance/electron-ground.md`.

## Source preparation

Run through devenv:

```sh
node tools/accessibility/electron/prepare-source.mjs --prepare
node tools/accessibility/electron/prepare-source.mjs --sync
```

Preparation checks the exact Electron and depot_tools revisions and Electron
DEPS hash. Sync uses the exact Chromium commit, disables depot_tools updates,
and records resolved dependency revisions. It does not apply the producer or
claim a successful build.

## Runtime acceptance fixture

`fixture.cjs` creates two identically titled native windows with separate inputs,
buttons, numeric values and selection controls. It requires the private test bus
and an explicitly supplied disposable profile. It accepts only fixture lifecycle
commands on stdin. No authenticated application or remote page is opened.

After building and packaging the patched runtime with its matching bridge,
set `DOJO_TEST_ELECTRON_RUNTIME` to its absolute launch path when running
`tools/speech/check-semantic-accessibility-live.mjs --private-bus`. The optional
branch checks runtime versions, exact roots, four semantic actions, sibling
isolation, accessibility disable/re-enable, bridge cleanup/reinitialization,
hide/show and root recreation. That branch is not counted as passing
until it executes against the compiled producer.

## Local build

After dependency sync finishes, run these in order through devenv:

```sh
node tools/accessibility/electron/build.mjs --hooks
node tools/accessibility/electron/build.mjs --configure
node tools/accessibility/electron/build.mjs --objects
node tools/accessibility/electron/build.mjs --compile
```

The hook step appends the exact producer as the final patch in Electron's own
Chromium patch series. Build arguments disable remote execution and select the
pinned checkout's GN and Ninja binaries. Compilation runs at lower CPU priority
with four jobs by default (DOJO_ELECTRON_JOBS overrides). The object step compiles the six changed native integration
objects; it does not claim to have built an Electron runtime. The compile step
builds the actual runtime distribution. Resolved dependency revisions are
recorded in `resolved-source-revisions.json` and checked before the hooks run.

Build the native bridge lifecycle addon with:

```sh
nix-build tools/accessibility/atk-bridge/build.nix --argstr target addon --no-out-link
```

Set `DOJO_TEST_ELECTRON_BRIDGE_ADDON` to the resulting
`lib/dojo-atk-bridge-fixture.node` as well as setting the packaged runtime path.
The isolated fixture uses that addon to clean up and reinitialize its own ATK
bridge. Acceptance requires both window annotations to withdraw, old references
to refuse, and both exact trees to return under the new bus owner with their
application values preserved. These Electron checks remain pending until the
actual compiled runtime executes them.

## Package the compiled runtime

`runtime-package.nix` adapts the pinned Nixpkgs binary recipe instead of
recreating its library closure. The original bytes, license and reproducing
patch are retained beside the recipe. The adaptation accepts the compiled
archive, uses the new ATK bridge, handles Electron 44's omitted ANGLE stubs,
and clears inherited loader overrides in the per-process launcher. GApps theme
settings remain in the upstream wrapper.

After the compile step produces `dist.zip`, package that exact archive and its
reported SHA256:

```sh
nix-build tools/accessibility/electron/runtime-build.nix --no-out-link \
  --arg distribution /absolute/path/to/out/Dojo/dist.zip \
  --argstr distributionSha256 THE_ACTUAL_BUILD_SHA256
```

The input is copied into the immutable store before its digest is checked.
The recipe has passed Nix parsing and its patch reproduces from the pinned
original. It has not yet packaged or launched the custom runtime; that requires
the unfinished full Electron build and subsequent live acceptance.
