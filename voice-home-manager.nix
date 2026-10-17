{ config, pkgs, lib, ... }:
let
  home = config.home.homeDirectory;
in
{
  # The unit template names the checkout and the devenv profile; fill them in for this account.
  xdg.configFile."systemd/user/dojo-voice.service".text = builtins.replaceStrings
    [ "@DOJO_ROOT@" "@HOME@" ] [ config.dojo.root home ]
    (builtins.readFile ./tools/speech/dojo-voice.service);
  home.activation.dojoVoiceService = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    ${pkgs.systemd}/bin/systemctl --user daemon-reload
    ${pkgs.systemd}/bin/systemctl --user enable --now dojo-voice.service
  '';
  home.activation.dojoNiriConfigCheck = lib.hm.dag.entryAfter [ "linkGeneration" ] ''
    (
      source ${home}/.local/share/dojo/niri/current/environment.sh
      ${home}/.local/share/dojo/niri/current/niri validate --config ${config.xdg.configHome}/niri/config.kdl
    )
  '';
  home.activation.dojoVoiceMcp = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    ${pkgs.python3}/bin/python3 ${config.dojo.root}/scripts/install-voice-config.py
  '';
}
