{ target ? "bridge" }:
let
  lock = builtins.fromJSON (builtins.readFile ../../../devenv.lock);
  input = lock.nodes.${lock.nodes.root.inputs.nixpkgs}.locked;
  source = builtins.fetchTree { inherit (input) type owner repo rev narHash; };
  pkgs = import source.outPath { system = "x86_64-linux"; };
  targets = {
    bridge = ./default.nix;
    fixture = ./fixture.nix;
    addon = ./fixture-addon.nix;
  };
in assert builtins.hasAttr target targets;
  import targets.${target} { inherit pkgs; }
