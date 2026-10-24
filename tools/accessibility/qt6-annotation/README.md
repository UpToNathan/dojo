# Qt 6.12.0 accessibility annotations

This generic Qt plugin associates each visible native Wayland QWindow with that
window's existing AT-SPI root. It uses the built-in accessibility bridge's actual
connection and QWindow::accessibleRoot(). It does not select a window by PID,
name, title, bounds, or enumeration order.

The plugin requires the pinned Qt 6.12.0 package and its private ABI. CMake
requires the exact version; runtime setup also checks the Qt library directory.
The source pins and copied protocol hash are in provenance.json. The small
AtSpiAdaptor path conversion and annotation publication follow the pinned Qt
sources and change 717474. The generated Wayland bindings use MR493's unmodified
XML. The plugin code is distributed under LGPL-3.0-only. Qt5 builds this same
lifecycle policy with a versioned authoritative-connection accessor branch.

QWaylandClientExtension binds the protocol through Qt's existing registry and
dispatch. The plugin creates no display connection, reader thread, roundtrip,
or event dispatcher. All native-window work runs on the GUI thread. Before
publishing, an asynchronous GetRole request on the existing bridge connection
confirms that its owner exports that exact object. A generation check discards
replies after lifetime changes. This is observation and identity publication;
the plugin sends no accessibility actions.

Activation observers revoke annotations on accessibility disable. Native
wl_surface destruction revokes them directly. Creation and visibility events
schedule reconciliation. A 250 ms reconciliation timer also detects root and
bus-owner changes and retries unavailable roots. Destruction cancels pending
publication by changing its generation. Duplicate plugin keys create one
producer. A compositor without the pinned draft protocol gets no annotations.

## Build and launch

Import qt6-annotation-home-manager.nix and enable
programs.dojoQt6Annotation.enable to build the package. The maintained
home-manager.nix imports and enables it. This adds session environment setup for
future launches and a dojo-with-qt6-annotation command. It does not activate Home
Manager or restart existing applications itself.

For a local build, run CMake inside devenv with the pinned Qt6 prefix and CMake,
pkg-config, and wayland-scanner available:

```
devenv shell -- cmake -S tools/accessibility/qt6-annotation -B /tmp/dojo-qt6-annotation-build -DCMAKE_PREFIX_PATH="$QT6_PREFIX"
devenv shell -- cmake --build /tmp/dojo-qt6-annotation-build -j4
```

The plugin is under the build directory's plugins/generic. Launch the fixture
with QT_PLUGIN_PATH including that plugins directory and
QT_QPA_GENERIC_PLUGINS including dojo-qt6-annotation-6.12.0. Preserve existing
values. QT_QPA_PLATFORM=wayland selects the native backend. Enable AT-SPI with
the session's accessibility settings or QT_LINUX_ACCESSIBILITY_ALWAYS_ON=1.

The fixture opens two windows in one process. Each has a text field and a
button that destroys and recreates its native surface. --exercise-lifecycle
checks distinct accessibility roots, checks the requested plugin actually
initialized, recreates a native surface, toggles accessibility activation, and
exits. It prints whether the compositor advertises the annotation global.
That diagnostic does not establish that the compositor accepted both roots.

## Deployment limits

QT_PLUGIN_PATH and QT_QPA_GENERIC_PLUGINS additions preserve existing values.
The plugin's directory contains only a generic plugin; themes, styles and
platform backends continue using their existing paths. Session environment.d
and shell setup cover future processes after normal session environment reload.
Already running applications require a later restart.

Qt5 rejects Qt6 plugin metadata. When both maintained producers are installed,
each plugin recognizes the other major's launch key as a no-op, so a shared
session list does not produce a missing-key warning on supported runtimes.
The Qt6 producer itself does not bind Qt5 accessibility.
Bundled, sandboxed, differently built, or differently versioned Qt6 runtimes do
not gain support automatically. The runtime package directory guard deliberately
refuses a different Qt build even if it reports 6.12.0. Electron and GPUI clients
need their own producer.

A successful build and fixture run prove loading and fixture lifecycle only.
Exact binding requires an end-to-end run against the matching compositor
consumer, checking separate roots on both native surfaces and revocation after
recreation, disable, destruction, and bus loss. Bus loss is detected by the
reconciliation timer and compositor owner validation; this is not an atomic
snapshot across the two processes.
