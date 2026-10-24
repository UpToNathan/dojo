{ config, pkgs, lib, ... }:
let
  home = config.home.homeDirectory;
in
{
  # The unit template names the checkout; fill it in for this account.
  xdg.configFile."systemd/user/dojo-voice.service".text = builtins.replaceStrings
    [ "@DOJO_ROOT@" "@HOME@" ] [ config.dojo.root home ]
    (builtins.readFile ./tools/speech/dojo-voice.service);
  xdg.configFile."systemd/user/dojo-voice.socket".source = ./tools/speech/dojo-voice.socket;
  xdg.configFile."systemd/user/dojo-voice-mic.service".text = builtins.replaceStrings
    [ "@DOJO_ROOT@" ] [ config.dojo.root ]
    (builtins.readFile ./tools/speech/dojo-voice-mic.service);
  # The agent runtime is socket activated: systemd holds dojo-voice.sock and starts
  # dojo-voice.service on the first connection, with or without a microphone, and the runtime exits
  # after ten idle minutes. The watcher tells a running runtime when voice input can run; it is the
  # only part that follows the microphone and it never starts the runtime.
  home.activation.dojoVoiceService = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    systemctl=${pkgs.systemd}/bin/systemctl
    $systemctl --user daemon-reload
    # The service no longer starts at login; drop the old link, and the one the old units made.
    $systemctl --user disable dojo-voice.service 2>/dev/null || true
    $systemctl --user enable dojo-voice.socket dojo-voice-mic.service
    # A runtime started the old way owns the socket file, which the socket unit then cannot bind.
    # Stop it once; socket activation starts it again when it is needed.
    if ! $systemctl --user is-active --quiet dojo-voice.socket; then
      $systemctl --user stop dojo-voice.service
    fi
    $systemctl --user start dojo-voice.socket
    $systemctl --user restart dojo-voice-mic.service
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
