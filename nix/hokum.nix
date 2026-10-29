{
  autoPatchelfHook,
  lib,
  libGL,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxkbcommon,
  pkg-config,
  rustPlatform,
  stdenv,
  vulkan-loader,
  wayland,
}:

# Hokum, built from vendor/hokum and the Rune it draws with (vendor/rune), both Git subtrees
# with Dojo's changes: a blur behind glass cards, installed font families, and no update
# offers for a store path. See docs/notifications.md.
rustPlatform.buildRustPackage {
  pname = "hokum";
  version = "0.1.2";

  src = lib.fileset.toSource {
    root = ../vendor;
    fileset = lib.fileset.difference
      (lib.fileset.unions [ ../vendor/hokum ../vendor/rune ])
      (lib.fileset.unions [
        (lib.fileset.maybeMissing ../vendor/hokum/target)
        (lib.fileset.maybeMissing ../vendor/rune/target)
      ]);
  };
  sourceRoot = "source/hokum";
  cargoLock.lockFile = ../vendor/hokum/Cargo.lock;

  nativeBuildInputs = [ autoPatchelfHook pkg-config ];
  buildInputs = [ libxkbcommon stdenv.cc.cc.lib wayland ];
  # winit and wgpu open these with dlopen, so they are not in the executable's NEEDED list.
  runtimeDependencies = [
    libGL
    libx11
    libxcb
    libxcursor
    libxi
    libxkbcommon
    vulkan-loader
    wayland
  ];

  doCheck = false;

  meta = {
    description = "Freedesktop notification daemon for Wayland";
    homepage = "https://hokum.app";
    license = lib.licenses.mit;
    mainProgram = "hokum";
    platforms = [ "x86_64-linux" ];
  };
}
