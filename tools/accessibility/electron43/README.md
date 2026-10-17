# Electron 43 native Wayland producer

This is the version-specific source patch for the installed Legcord runtime's
Electron 43.6.0 / Chromium 150.0.7871.250 pair. It has not passed a full C++ build
or runtime fixture. No installed runtime or running application was changed.

Electron commit `3333109a2998815e64ffc0a0a54d9a82899fe512` pins that Chromium
version in DEPS. The Chromium tag resolves to
`f916f1b7136907af19d96b95b891533d091d932a`.

Apply `chromium-wayland-atspi-annotation.patch` after Electron's Chromium patch
series. All 234 listed Electron patches were inspected for modified producer
paths. The only overlap is `fix_wayland_test_crash_on_teardown.patch`; clean
application after its exact bytes passed. Provenance pins DEPS, the patch list,
the overlapping patch, each Chromium preimage, and this producer patch.

The Electron 44 producer applies without changing its C++ additions or policy.
This patch regenerates the diff against Chromium 150's exact preimages. The
Widget root ATK expression, platform-window accessor, readiness and native API
mode checks retain the same signatures at this version. The protocol manager,
canonical XML, and bridge accessor contract are reused literally.

The producer requires the separately patched at-spi2-core export:

```c
gboolean dojo_atk_bridge_dup_accessible_reference(
    AtkObject *obj, gchar **owner, gchar **path);
```

Use an isolated runtime closure containing that bridge. Missing export means
no annotation. The producer runs on Chromium's UI thread, binds through its
existing Wayland registry, and requests its existing compositor frame pipeline.
A 250 ms timer reconciles root, accessibility mode, visibility, and bus identity.
Hide and native destruction revoke directly. It uses the same window's actual
root object and native surface; it performs no title, PID, or geometry join.

Check a Chromium checkout before applying:

```sh
node tools/accessibility/electron43/check-producer-patch.mjs /path/to/chromium/src
```

Run the source fixture using a git checkout whose HEAD contains the exact
upstream preimages:

```sh
node tools/accessibility/electron43/test-producer-patch.mjs /path/to/chromium/src
```

The fixture proves clean application, the Electron overlap, drift refusal, and
already-applied refusal in disposable copies. It does not compile Chromium.
Next build this source pair with Electron's dependencies, then run the separate
two-window fixture in isolated nested Niri. Native root identity, frame commit,
mode disable/re-enable, and accessibility-bus lifecycle still need that proof.

`CHROMIUM-LICENSE` is the literal license at the pinned Chromium commit. The
canonical protocol retains its copyright and license inside the patch.
