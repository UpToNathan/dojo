{ config, pkgs, ... }:
{
  home.packages = [
    (pkgs.writeShellScriptBin "dojo-update" ''
      exec ${pkgs.python3}/bin/python3 ${config.dojo.root}/scripts/dojo-update "$@"
    '')
  ];
}
