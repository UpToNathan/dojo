{ pkgs, ... }:
{
  home.packages = [
    (pkgs.writeShellScriptBin "dojo-update" ''
      exec ${pkgs.python3}/bin/python3 /home/nathan/Dojo/scripts/dojo-update "$@"
    '')
  ];
}
