# Exact Electron Wayland accessibility producer

This is a source checkpoint, not a built or deployed Electron producer. No T3
window, application content, host library, or running runtime was changed.

## Actual runtime and source pins

T3's probed runtime is Electron 44.4.2 / Chromium 152.0.7977.130. Its ELF SHA256
is `88ac601406f5847fdda5aa86fceb2fa9ab31ce26bc64f777aca27c5c5421998f`.
The repeatable probe and accepted runtime identity live in
`tools/accessibility/probe-electron-runtime.mjs` and
`provenance/native-electron-investigation.json`.

Electron's annotated v44.4.2 tag resolves to commit
`34a45a290c7debc4907937cbb77509759247cebb`. Its DEPS explicitly pins
Chromium 152.0.7977.130, whose tag resolves to commit
`2c592105bbcd9490a9894df48d0fe59b2c512651`.
[Electron DEPS](https://github.com/electron/electron/blob/34a45a290c7debc4907937cbb77509759247cebb/DEPS)

The inspected T3 processes map `/usr/lib/libatk-bridge-2.0.so.0.0.0`, GTK3,
and Wayland client libraries. The installed host package reports at-spi2-core
2.62.1. Its libatk-bridge ELF SHA256 is
`502a1780ffced063b1eee39430798a927f160e4871e563dbfaabd976665787c9`.
GNOME's 2.62.1 tag resolves to commit
`3a214cddffbf345b8c0d0b17e458abbc5ed851b1`. This upstream tag and installed
package version identify the inspected API implementation; a distro package
can carry patches, so the actual mapped ELF hash remains a separate pin.

Legcord's Electron 43.6.0 / Chromium 150.0.7871.250 needs its own source pair
and build. The older 43.6.0 research is not the T3 pin.

## Confirmed ownership chain

The producer can live entirely in Chromium's Linux Views/Ozone integration.
Electron-specific BrowserWindow lookup is unnecessary.

1. `DesktopWindowTreeHostLinux` inherits `DesktopWindowTreeHostPlatform`, which
   provides protected `Widget* GetWidget()`. Its `OnNativeWidgetCreated` already
   has the actual platform-host lifetime. This gives the host's own Views
   widget, without searching a registry by title, PID, bounds, or app ID.
2. The widget's `GetRootView()->GetNativeViewAccessible()` returns the actual
   exported ATK object for that window. Chromium's `AuraLinuxApplication`
   enumerates its registered widgets using this exact expression in
   `ChildAtIndex`. The application root and window root are distinct objects.
3. The same host inherits `aura::WindowTreeHostPlatform`, whose public
   `ui::PlatformWindow* platform_window()` returns its owned platform window.
4. On native Wayland that concrete object is `ui::WaylandWindow`, derived from
   `PlatformWindow`. Its `root_surface()` returns its own `WaylandSurface`;
   `WaylandSurface::surface()` returns that object's real `wl_surface*`.
5. A new typed `PlatformWindow` endpoint setter can pass the already determined
   AT-SPI owner/path directly to that platform window. The Wayland override
   attaches the canonical MR493 annotation to its own root surface. No reverse
   lookup from an opaque Electron window number is needed.

These inspected declarations and implementations establish the chain:

- [Desktop Linux host](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/views/widget/desktop_aura/desktop_window_tree_host_linux.cc)
- [Desktop platform host GetWidget](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/views/widget/desktop_aura/desktop_window_tree_host_platform.h)
- [Views ATK application/window roots](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/views/accessibility/view_ax_platform_node_delegate_auralinux.cc)
- [Aura platform_window accessor](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/aura/window_tree_host_platform.h)
- [PlatformWindow interface](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/platform_window/platform_window.h)
- [WaylandWindow ownership](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/ozone/platform/wayland/host/wayland_window.h)
- [WaylandWindow initialization/lifetime](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/ozone/platform/wayland/host/wayland_window.cc)
- [WaylandSurface surface()/Commit()](https://chromium.googlesource.com/chromium/src/+/2c592105bbcd9490a9894df48d0fe59b2c512651/ui/ozone/platform/wayland/host/wayland_surface.h)

## Exact ATK endpoint through the pinned bridge accessor

The pinned bridge exports private `spi_object_append_reference`, which leases
the supplied ATK object and serializes the bridge's own unique bus owner and
registered object path into a local unsent D-Bus message. That establishes the
object-to-endpoint join, but direct external use is unsafe during bridge
teardown and reconnect. Chromium's readiness boolean can remain true while the
bridge's bus is null.
[Exact serialization](https://github.com/GNOME/at-spi2-core/blob/3a214cddffbf345b8c0d0b17e458abbc5ed851b1/atk-adaptor/object.c#L93)
[Bridge lifecycle](https://github.com/GNOME/at-spi2-core/blob/3a214cddffbf345b8c0d0b17e458abbc5ed851b1/atk-adaptor/bridge.c)

The implementation therefore depends on this narrow export added inside the
pinned at-spi2-core bridge:

```c
gboolean dojo_atk_bridge_dup_accessible_reference(
    AtkObject *obj, gchar **owner, gchar **path);
```

Its implementation checks bridge initialization, owner thread, and the live
bus before calling the internal serializer and parsing the actual tuple. The
caller frees both outputs with `g_free`. The bridge patch also checks that a
cached object ID belongs to the current registry after cleanup/reinitialization.
The main worker owns that patch and its private-bus lifecycle fixture.

Chromium resolves only this optional export with `dlsym`. Missing export
means no annotation. It never reads bridge globals or private structure layout.
It also checks its existing `IsAtSpiReady()` and
`AXPlatform::GetInstance().GetMode().has_mode(AXMode::kNativeAPIs)` before
requesting a reference. The accessor remains the authoritative lifecycle guard.
Its call executes on the existing Views UI thread. The same pinned upstream
ATK utility already excludes calls into external DSOs from CFI indirect-call
instrumentation; the producer uses that exact exclusion on its callback.

## Why a preload or public Node addon does not provide the full chain

Electron's public `getNativeWindowHandle` returns
`NativeWindowViews::GetNativeWindowHandle()`, which returns
`GetAcceleratedWidget()`. On this exact Chromium Wayland backend,
`WaylandWindow` initializes that integer with
`WaylandWindowManager::AllocateAcceleratedWidget()`. The actual root surface is
a separate owned object. Casting this integer to `wl_surface*` is invalid.
[Actual Electron handle implementation](https://github.com/electron/electron/blob/34a45a290c7debc4907937cbb77509759247cebb/shell/browser/native_window_views.cc)

An Electron preload does not expose the Views widget, its ATK object, or its
Wayland display/surface. A public Node native addon can call available C APIs,
but the inspected executable's dynamic symbol table exposes no matching
`NativeWindowViews`, `ViewAccessibility`, `AXPlatformNode`, or `WaylandWindow`
C++ entry points. An addon cannot obtain the missing object links by passing
only a JS window ID. V8/DOM accessibility snapshots are not the native ATK
address and are outside the requested native producer route.

No evidence was found for a literal drop-in extension that publishes this join
for stock Electron. A native Chromium/Electron runtime patch is the concrete
route. Binary hooking, offset lookup, and global LD_PRELOAD interposition are
not part of this design.

## Implemented source patch and lifecycle

`tools/accessibility/electron/chromium-wayland-atspi-annotation.patch` contains
the concrete source changes. Its adjacent provenance records exact preimage
SHA256 hashes and the patch hash. The patch changes the host, platform-window
interface, Wayland registry, Wayland window, object traits, and protocol GN
targets. The manager follows the pinned `org_kde_kwin_appmenu` implementation.
The XML is a byte-for-byte copy of canonical MR493 commit
`2c4da1cf132e7084e0de8a4a7db4d3af7644b91d`.

A 250 ms UI-thread timer reconciles the actual root object, bridge owner/path,
visibility, and native accessibility mode. Changes replace the annotation;
missing readiness or reference revokes it. Hide and destruction revoke the
annotation directly. Native root surface ownership does not change across
hide/show in this pinned WaylandWindow implementation. Each new native window
gets its own annotation. The timer republishes after show or bridge reconnect.

Metadata changes request `Compositor::ScheduleFullRedraw()` and use Chromium's
existing frame/commit pipeline. They do not insert arbitrary wl_surface
commits or dispatch another Wayland queue. Revocation flushes the destructor
request through the existing connection. Annotation ownership ends before its
native surface is destroyed.

Verification so far covers exact preimage hashes, clean patch application after
Electron's single overlapping Wayland teardown fix, and compilation of the
canonical protocol's generated C implementation. These do not prove the C++
producer compiles or that a redraw publishes the annotation in every lifecycle
state. Those require the real Chromium build and isolated two-window runtime
fixture. Mode and bus changes can take up to one timer interval to reconcile;
hide and native window destruction have direct revocation paths.

## Build and verification route

The exact Electron source is cached at
`/home/nathan/.cache/dojo/upstream/electron/34a45a290c7debc4907937cbb77509759247cebb`.
Use that revision's DEPS with Electron's documented gclient checkout, sync, and
patch application process. Put Chromium edits in Electron's Chromium patch
series rather than modifying an unrelated Chromium main checkout. Pin every
additional build input and record the applied patch hashes.
[Build workflow at the actual Electron pin](https://github.com/electron/electron/blob/34a45a290c7debc4907937cbb77509759247cebb/docs/development/build-instructions-gn.md)

Build a custom Electron 44.4.2 runtime, with its own isolated launch closure and
pinned ATK bridge dependency. Reuse the T3 application resources only after
compatibility validation; keep its installed/running runtime intact during the
build. Build size is work to perform, not a feasibility blocker.

The bridge worker is proving the headless own-object/private-bus lifecycle
fixture. The next producer unit is to compile this real source patch and
run a separate two-window Electron fixture in the existing isolated nested
Niri session. Prove distinct exact roots, root and surface recreation,
accessibility disable/re-enable, bus owner loss, and ordinary typed semantic
operations. Public runtime replacement or T3 restart follows only after those
results. No Electron producer coverage is claimed by this checkpoint.

### Runtime probe safety

The runtime identity probe now requires one complete V1 Electron fuse wire with RunAsNode enabled before it executes the binary. It clears NODE_OPTIONS for the version query. The fuse format is pinned to electron/fuses 2.1.2 at 67d4eed3a8da0563c9afeac3229c09498d326435, with original source bytes, license, and integrity in tools/accessibility/electron-fuses/provenance.json.

Read-only ELF inspection found T3 enables RunAsNode and installed 1Password disables it. 1Password was not launched. Seven execution-boundary checks passed through devenv, including a positive control that reaches the intercepted execution function only for an enabled wire. The full Electron runtime distribution build remains in progress; no Electron GUI acceptance coverage is claimed.

### Native session inventory

The read-only `audit-native-bindings.mjs` run recorded the current T3 process and
its loaded GTK3/ATK/AT-SPI/Wayland libraries. The active compositor rejected
`AccessibilityTarget` with `error parsing request`, so the result correctly
remains unknown. The staged Niri build has not replaced the running compositor.
Evidence: `/tmp/dojo-native-binding-inventory.json`. No focus, input, title or
application-tree access was used.
