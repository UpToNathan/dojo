# Qt 5.15.19 accessibility annotations

Qt5 lacks an exported accessor for the built-in AT-SPI bridge's connection.
The small qtwayland-atspi-connection.patch adds one in the existing
QWaylandIntegration implementation, which already owns that bridge. The
accessor copies its actual QDBusConnection into caller-owned storage, requires
the GUI thread and an active bridge, and rejects an unavailable bus.
It does not inspect object memory or guess an application from its process.

The Nix module rebuilds QtWayland, including its platform plugins. QtBase and
its other consumers do not need source changes. The plugin and two-window
fixture compile the shared Qt6 producer policy with a Qt5 connection branch.
Native window/root identity, asynchronous root verification, generation
checks, activation handling, and native-surface lifetime handling are shared.
The Qt5 root path conversion is identical in the pinned Qt5 AtSpiAdaptor.
The canonical MR493 XML and its hash are shared without another copy.

The private ABI is pinned to Qt 5.15.19. The plugin also checks its actual Qt
library directory. It looks up only the versioned
`dojo_qt51519_accessibility_connection` accessor. An unpatched QtWayland runtime
cannot publish bindings through this plugin.

Import qt5-annotation-home-manager.nix and enable
programs.dojoQt5Annotation.enable to build the patched backend, plugin,
fixture, and dojo-with-qt5-annotation launcher. The maintained Home Manager config imports and enables this module for future
launches. It adds the generic-plugin path and key without replacing existing
values. It sets a QPA gate directory only when no explicit platform-plugin path
already exists. No activation or application restart occurs during a build.
An application that explicitly forces a different platform-plugin path remains
unsupported.

The QPA gate checks the exact Qt version and library directory before loading
the patched Wayland platform plugin by absolute path. It has no QtWayland
dependency. Rejected builds return to Qt's normal platform loader. This is
needed because Qt5's own plugin metadata check compares major/minor and ignores
patch versions. The patched backend root enters Qt's search paths only after
the gate accepts the runtime. Themes and styles keep their existing paths.

Qt5's direct platform-plugin loader tries the gate before the normal paths,
including paths prepended by Nix application wrappers. Matching system Qt5
applications can therefore load automatically after session configuration and
restart. Already running, bundled, sandboxed, or differently built runtimes do
not gain this coverage. Copying the generic plugin alone is insufficient.

The fixture accepts --exercise-lifecycle to check same-process distinct roots,
plugin initialization, native surface recreation, and activation disable/enable.
A run without the compositor's draft annotation global can prove loading and
lifecycle only. Exact bindings still require checking both published native
surfaces against the compositor consumer.

The paired Qt5/Qt6 plugins recognize the other major's launch key as a no-op.
This permits one additive session list without missing-key diagnostics on the
supported builds. Only the native major's own key creates a producer.
