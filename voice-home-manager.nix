{ pkgs, lib, ... }:
{
  xdg.configFile."systemd/user/dojo-voice.service".source = ./tools/speech/dojo-voice.service;
  home.activation.dojoVoiceService = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    ${pkgs.systemd}/bin/systemctl --user daemon-reload
    ${pkgs.systemd}/bin/systemctl --user enable --now dojo-voice.service
  '';
  home.activation.dojoNiriConfigCheck = lib.hm.dag.entryAfter [ "linkGeneration" ] ''
    (
      source /home/nathan/.local/share/dojo/niri/current/environment.sh
      /home/nathan/.local/share/dojo/niri/current/niri validate --config /home/nathan/.config/niri/config.kdl
    )
  '';
  home.activation.dojoVoiceMcp = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    ${pkgs.python3}/bin/python3 /home/nathan/Dojo/scripts/install-voice-config.py
  '';
}
