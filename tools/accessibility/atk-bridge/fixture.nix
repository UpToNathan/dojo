{ pkgs }:
let bridge = import ./default.nix { inherit pkgs; };
in pkgs.stdenv.mkDerivation {
  pname = "dojo-atk-endpoint-fixture";
  version = "1";
  src = ./fixture.c;
  dontUnpack = true;
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ bridge pkgs.dbus ];
  buildPhase = ''
    $CC -Wall -Wextra -Werror -Wno-unused-parameter "$src" -o fixture $(pkg-config --cflags --libs atk atk-bridge-2.0)
  '';
  installPhase = ''
    mkdir -p "$out/bin"
    cp fixture "$out/bin/dojo-atk-endpoint-fixture"
  '';
}
