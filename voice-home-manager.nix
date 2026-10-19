{ config, pkgs, lib, ... }:
let
  home = config.home.homeDirectory;
in
{
  # The unit template names the checkout; fill it in for this account.
  xdg.configFile."systemd/user/dojo-voice.service".text = builtins.replaceStrings
    [ "@DOJO_ROOT@" "@HOME@" ] [ config.dojo.root home ]
    (builtins.readFile ./tools/speech/dojo-voice.service);
  xdg.configFile."systemd/user/dojo-voice-mic.service".text = builtins.replaceStrings
    [ "@DOJO_ROOT@" ] [ config.dojo.root ]
    (builtins.readFile ./tools/speech/dojo-voice-mic.service);
  # The watcher starts dojo-voice.service when a microphone appears and stops it when the last
  # one goes, so the voice service is no longer enabled at login itself.
  home.activation.dojoVoiceService = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    rm -f ${config.xdg.configHome}/systemd/user/graphical-session.target.wants/dojo-voice.service
    ${pkgs.systemd}/bin/systemctl --user daemon-reload
    ${pkgs.systemd}/bin/systemctl --user enable dojo-voice-mic.service
    ${pkgs.systemd}/bin/systemctl --user restart dojo-voice-mic.service
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
