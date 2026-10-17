{ config, lib, pkgs, ... }:
let
  cfg = config.programs.dojoNativeAccessibility;
in {
  options.programs.dojoNativeAccessibility.enable =
    lib.mkEnableOption "native toolkit accessibility for Dojo assistance";

  config = lib.mkIf cfg.enable {
    dconf.enable = true;
    dconf.settings."org/gnome/desktop/interface".toolkit-accessibility = true;
    home.file.".local/bin/dojo-agent".source = pkgs.writeShellScript "dojo-agent" ''
      exec ${config.dojo.root}/scripts/dojo-agent "$@"
    '';
  };
}
