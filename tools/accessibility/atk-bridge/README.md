# Exact ATK endpoint accessor

This fork of at-spi2-core 2.62.1 adds
`dojo_atk_bridge_dup_accessible_reference(AtkObject *, gchar **, gchar **)`.
It returns the object's actual AT-SPI unique bus owner and registered path.
Both output strings belong to the caller and must be released with `g_free`.
On refusal both outputs are null. Call it on the thread that initialized the
bridge, with a live object owned by the caller.

The implementation runs inside the bridge and checks its actual initialization,
connection and registry state. It reuses `spi_object_append_reference` to export
the exact object reference into an unsent local D-Bus message. Export can lease
and lazily register the object. It does not invoke an accessibility action.
Uninitialized, disconnected, cleaned-up, defunct and foreign-thread calls refuse.

The patch also fixes a stale-ID case in the upstream object registry. After
cleanup and reinitialization an existing ATK object can retain its old numeric
ID even though the new registry has no corresponding object. The lookup now
checks the current registry before reusing that ID. The private-bus fixture
failed on this lifecycle case before the fix and passed afterward.

`default.nix` builds the source revision and patch recorded in `provenance.json`.
It inherits the repository's locked Nixpkgs build dependencies. It installs the
upstream LGPL 2.1 license with the library. `fixture.nix` builds a headless native
fixture against that exact library.

Build through the repository's locked Nixpkgs input:

```sh
nix-build tools/accessibility/atk-bridge/build.nix --no-out-link
nix-build tools/accessibility/atk-bridge/build.nix --argstr target fixture --no-out-link
```

Run the actual bus check with the resulting store paths:

```sh
node tools/accessibility/check-atk-endpoint-live.mjs \
  /nix/store/gi07cpdrvrg21nm5iizh5m9h5yhjjsp1-dojo-atk-endpoint-fixture-1/bin/dojo-atk-endpoint-fixture \
  /nix/store/fm0m2fyrcrssgbl771pm0m96ivvpzdgj-dojo-at-spi2-core-2.62.1
```

The runner creates a private runtime and session bus with only the built
accessibility launcher available for activation. It uses memory settings,
removes display and inherited dynamic-library overrides, and opens no GUI.
It reads each exported object's actual Name over AT-SPI, checks distinct paths,
rejects uninitialized/defunct/foreign-thread access, cleans up the bridge,
checks retirement of the old owner, reinitializes, and reads the same two live
objects from the new owner.

This verifies the endpoint accessor. It does not prove Electron window binding.
The Chromium producer must be compiled and tested with its own exact window
roots and Wayland surfaces before deployment. No host bridge was replaced.

`fixture-addon.nix` builds a small N-API 8 addon for the isolated Electron
fixture. It exposes only the upstream bridge initialization and cleanup calls,
so the fixture can exercise its own real bridge lifecycle on Electron's UI
thread. The addon built successfully and loaded in Node, where cleanup was a
safe no-op before bridge initialization. That load check is not an Electron
producer test. The addon is not loaded into the user's application or profile.
