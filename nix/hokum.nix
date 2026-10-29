{
  autoPatchelfHook,
  fetchurl,
  lib,
  libGL,
  libx11,
  libxcb,
  libxcursor,
  libxi,
  libxkbcommon,
  stdenv,
  stdenvNoCC,
  vulkan-loader,
  wayland,
}:

stdenvNoCC.mkDerivation {
  pname = "hokum";
  version = "0.1.1";

  src = fetchurl {
    url = "https://github.com/anomalyco/hokum/releases/download/v0.1.1/hokum-0.1.1-linux-x64";
    hash = "sha256-BaYiAhyQwqDgyN7USzTtm7NKehAmwP9ivFiCy4bQZC8=";
  };

  dontUnpack = true;
  dontBuild = true;
  nativeBuildInputs = [ autoPatchelfHook ];
  buildInputs = [ libxkbcommon stdenv.cc.cc.lib ];
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

  installPhase = ''
    runHook preInstall
    install -Dm0555 "$src" "$out/bin/hokum"
    runHook postInstall
  '';

  meta = {
    description = "Freedesktop notification daemon for Wayland";
    homepage = "https://hokum.app";
    license = lib.licenses.mit;
    mainProgram = "hokum";
    platforms = [ "x86_64-linux" ];
  };
}
