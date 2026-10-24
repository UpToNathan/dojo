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
  # The agent runtime (dojo-voice.service) runs at login on every host, with or without a
  # microphone. The watcher tells it when voice input can run; it is the only part that follows
  # the microphone. `start` does not restart a runtime that is already running, so an upgrade
  # takes effect on the next login or `systemctl --user restart dojo-voice.service`.
  home.activation.dojoVoiceService = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    ${pkgs.systemd}/bin/systemctl --user daemon-reload
    ${pkgs.systemd}/bin/systemctl --user enable dojo-voice.service dojo-voice-mic.service
    ${pkgs.systemd}/bin/systemctl --user start dojo-voice.service
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
