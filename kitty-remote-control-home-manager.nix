# Dojo's kitty remote-control password: per machine, outside the Nix store, readable by the user
# only, and allowed nothing but `ls`. kitty's own settings (password mode, private socket, the
# include) are in home-manager.nix. Made once; delete the file to make a new one.
{ config, pkgs, lib, ... }:
{
  home.activation.dojoKittyRemoteControl = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    file="${config.xdg.configHome}/kitty/dojo-remote-control.conf"
    if [ ! -s "$file" ]; then
      run mkdir -p "$(dirname "$file")"
      secret=$(${pkgs.coreutils}/bin/head -c 32 /dev/urandom | ${pkgs.coreutils}/bin/base64 | ${pkgs.coreutils}/bin/tr -dc 'A-Za-z0-9')
      if [ -z "''${DRY_RUN:-}" ]; then
        (umask 077; printf 'remote_control_password "%s" ls\n' "$secret" > "$file.new") && mv -f "$file.new" "$file"
      fi
    fi
  '';
}
