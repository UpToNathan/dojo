{ pkgs, distribution, distributionSha256 }:
let
  pins = builtins.fromJSON (builtins.readFile ../../../provenance/native-electron-investigation.json);
  immutableDistribution = builtins.path { path = distribution; name = "dojo-electron-${pins.electron.version}-dist.zip"; };
  bridge = import ../atk-bridge/default.nix { inherit pkgs; };
  packageBinary = pkgs.callPackage ./nixpkgs-binary.nix { at-spi2-atk = bridge; };
in
assert pkgs.stdenv.hostPlatform.system == "x86_64-linux";
assert builtins.match "[a-f0-9]{64}" distributionSha256 != null;
assert builtins.hashFile "sha256" immutableDistribution == distributionSha256;
(packageBinary pins.electron.version immutableDistribution).overrideAttrs (old: {
  pname = "dojo-electron-native";
  passthru = old.passthru // {
    inherit bridge;
    electronSourceRevision = pins.electron.revision;
    chromiumSourceRevision = pins.chromium.revision;
  };
})
