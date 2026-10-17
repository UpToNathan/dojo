{ pkgs ? import <nixpkgs> {} }:
pkgs.stdenv.mkDerivation {
  name = "dojo-agent";
  dontUnpack = true;
  buildPhase = ''$CC ${./agent-launcher.c} -o dojo-agent'';
  installPhase = ''install -m 0555 dojo-agent "$out"'';
}
