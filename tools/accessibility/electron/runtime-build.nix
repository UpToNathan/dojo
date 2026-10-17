{ distribution, distributionSha256 }:
let
  lock = builtins.fromJSON (builtins.readFile ../../../devenv.lock);
  input = lock.nodes.${lock.nodes.root.inputs.nixpkgs}.locked;
  source = builtins.fetchTree { inherit (input) type owner repo rev narHash; };
  pkgs = import source.outPath { system = "x86_64-linux"; };
in import ./runtime-package.nix { inherit pkgs distribution distributionSha256; }
