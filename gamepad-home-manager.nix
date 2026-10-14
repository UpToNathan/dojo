{ pkgs, ... }:
{
  systemd.user.services.dojo-gamepad = {
    Unit = {
      Description = "DualSense controls for Dojo";
      After = [ "graphical-session.target" ];
      PartOf = [ "graphical-session.target" ];
      ConditionEnvironment = "WAYLAND_DISPLAY";
    };
    Service = {
      ExecStart = "${pkgs.python3}/bin/python3 /home/nathan/Dojo/scripts/dojo-gamepad.py";
      Restart = "on-failure";
      RestartSec = 3;
      TimeoutStopSec = 10;
      KillMode = "mixed";
    };
    Install.WantedBy = [ "graphical-session.target" ];
  };
}
