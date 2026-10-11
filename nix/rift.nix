{
  autoPatchelfHook,
  fetchurl,
  gnutar,
  lib,
  stdenv,
  stdenvNoCC,
  zlib,
}:

stdenvNoCC.mkDerivation {
  pname = "rift";
  version = "0.0.13";

  src = fetchurl {
    url = "https://github.com/anomalyco/rift/releases/download/v0.0.13/rift-v0.0.13-x86_64-unknown-linux-gnu.tar.gz";
    hash = "sha256-Q6cBY5dCZiDAqX7meKPgJ6H0uD+4g3fMaxcJBHSr8F8=";
  };

  dontUnpack = true;
  dontBuild = true;
  nativeBuildInputs = [ autoPatchelfHook gnutar ];
  buildInputs = [ stdenv.cc.cc.lib zlib ];

  installPhase = ''
    runHook preInstall
    mkdir -p "$out/bin"
    tar -xzf "$src" -C "$out/bin" rift
    chmod 0555 "$out/bin/rift"
    runHook postInstall
  '';

  meta = {
    description = "Copy-on-write workspaces for coding agents";
    homepage = "https://github.com/anomalyco/rift";
    license = lib.licenses.mit;
    mainProgram = "rift";
    platforms = [ "x86_64-linux" ];
  };
}
