{ pkgs, ... }:
let
  rift = pkgs.callPackage ./nix/rift.nix { };
in
{
  home.packages = [ rift ];

  # Rift's V2 plugin is loaded by OpenCode from this pinned upstream source.
  # The executable override keeps the plugin tied to the same Nix package
  # instead of relying on whichever `rift` happens to be first on PATH.
  xdg.configFile."opencode/opencode.json".text = builtins.toJSON {
    "$schema" = "https://opencode.ai/config.json";
    plugins = [
      {
        package = "github:anomalyco/rift#v0.0.13::path:plugins/opencode";
        options = {
          executable = "${rift}/bin/rift";
          copyAll = false;
          hooks = true;
        };
      }
    ];
  };
}
