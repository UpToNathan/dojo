{ pkgs, inputs }:
let
  source = pkgs.applyPatches {
    name = "pi-dojo-source";
    src = inputs.pi.outPath;
    patches = [ ./pi-kimi-api.patch ];
  };
in
pkgs.callPackage (inputs.pi.outPath + "/nix/package.nix") {
  inherit source;
}
