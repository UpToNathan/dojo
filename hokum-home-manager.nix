# Hokum (https://hokum.app) is the session's notification daemon: it owns
# org.freedesktop.Notifications. The service and the D-Bus activation file below are the
# upstream packaging files (packaging/hokum.service, packaging/app.hokum.Notifications.service)
# pointed at the Nix package, so a notification sent before the graphical session has started
# the unit still reaches it. See docs/notifications.md.
{ pkgs, ... }:
let
  hokum = pkgs.callPackage ./nix/hokum.nix { };
in
{
  home.packages = [ hokum ];

  systemd.user.services.hokum = {
    Unit = {
      Description = "Hokum notification daemon (org.freedesktop.Notifications)";
      Documentation = "https://hokum.app";
      After = [ "graphical-session.target" ];
      PartOf = [ "graphical-session.target" ];
      Conflicts = [ "mako.service" "dunst.service" "swaync.service" ];
      StartLimitIntervalSec = 60;
      StartLimitBurst = 5;
    };
    Service = {
      Type = "dbus";
      BusName = "org.freedesktop.Notifications";
      ExecCondition = "/bin/sh -c '[ -n \"$WAYLAND_DISPLAY\" ]'";
      ExecStart = "${hokum}/bin/hokum service";
      Restart = "on-failure";
      RestartSec = 2;
    };
    Install.WantedBy = [ "graphical-session.target" ];
  };

  home.file.".local/share/dbus-1/services/app.hokum.Notifications.service".text = ''
    [D-BUS Service]
    Name=org.freedesktop.Notifications
    Exec=${hokum}/bin/hokum service
    SystemdService=hokum.service
  '';
}
