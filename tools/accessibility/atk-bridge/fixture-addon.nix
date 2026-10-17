{ pkgs }:
let bridge = import ./default.nix { inherit pkgs; };
in pkgs.stdenv.mkDerivation {
  pname = "dojo-atk-bridge-fixture-addon";
  version = "1";
  src = ./fixture-addon.c;
  dontUnpack = true;
  nativeBuildInputs = [ pkgs.pkg-config ];
  buildInputs = [ bridge ];
  buildPhase = ''
    $CC -shared -fPIC -Wall -Wextra -Werror -Wno-unused-parameter \
      -I${pkgs.nodejs}/include/node "$src" -o bridge.node \
      $(pkg-config --cflags --libs atk atk-bridge-2.0)
  '';
  installPhase = ''
    mkdir -p "$out/lib"
    cp bridge.node "$out/lib/dojo-atk-bridge-fixture.node"
  '';
}
