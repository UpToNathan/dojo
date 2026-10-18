{ config, lib, pkgs, ... }:
let
  portalRouting = ''
    [preferred]
    default=gtk
    org.freedesktop.impl.portal.FileChooser=termfilechooser
    org.freedesktop.impl.portal.Settings=darkman
    org.freedesktop.impl.portal.ScreenCast=gnome
    org.freedesktop.impl.portal.Screenshot=gnome
  '';
  backend = pkgs.xdg-desktop-portal-termfilechooser;
  fileManagerPython = pkgs.python3.withPackages (ps: [ ps.dbus-next ]);
  fileManager = pkgs.writeShellScriptBin "dojo-yazi-filemanager" ''
    export DOJO_KITTY=${lib.escapeShellArg "${config.programs.kitty.package}/bin/kitty"}
    export DOJO_YAZI=${lib.escapeShellArg "${config.programs.yazi.package}/bin/yazi"}
    exec ${fileManagerPython}/bin/python3 ${./scripts/yazi-filemanager.py}
  '';
  chooser = pkgs.writeShellScriptBin "dojo-yazi-chooser" ''
    export DOJO_KITTY=${lib.escapeShellArg "${config.programs.kitty.package}/bin/kitty"}
    export DOJO_YAZI=${lib.escapeShellArg "${config.programs.yazi.package}/bin/yazi"}
    export DOJO_YAZI_CHOOSER_PLUGIN=${./scripts/yazi-chooser.lua}
    exec ${pkgs.python3}/bin/python3 ${./scripts/yazi-chooser.py} "$@"
  '';
  # The session bus finds D-Bus service files through XDG_DATA_DIRS, which it reads once when it
  # starts. A login session that began before the Nix profile existed never sees the portals (so
  # no file picker) until the next login. ~/.local/share is always watched, so link the portal
  # services there and the desktop works whenever the session started.
  portalServiceFiles = lib.listToAttrs (lib.concatMap
    (entry: map
      (name: {
        name = "dbus-1/services/${name}.service";
        value.source = "${entry.package}/share/dbus-1/services/${name}.service";
      })
      entry.services)
    [
      {
        package = pkgs.xdg-desktop-portal;
        services = [
          "org.freedesktop.portal.Desktop"
          "org.freedesktop.portal.Documents"
          "org.freedesktop.impl.portal.PermissionStore"
        ];
      }
      { package = pkgs.xdg-desktop-portal-gtk; services = [ "org.freedesktop.impl.portal.desktop.gtk" ]; }
      { package = pkgs.xdg-desktop-portal-gnome; services = [ "org.freedesktop.impl.portal.desktop.gnome" ]; }
      {
        package = config.services.darkman.package;
        services = [ "nl.whynothugo.darkman" "org.freedesktop.impl.portal.desktop.darkman" ];
      }
    ]);
in
{
  # The parent retains ownership of Yazi/Kitty settings and portal routing.
  assertions = [
    { assertion = config.programs.yazi.enable; message = "Yazi chooser requires programs.yazi.enable"; }
    { assertion = config.programs.kitty.enable; message = "Yazi chooser requires programs.kitty.enable"; }
  ];
  xdg.configFile."xdg-desktop-portal/niri-portals.conf".text = portalRouting;
  xdg.configFile."xdg-desktop-portal/portals.conf".text = portalRouting;
  home.sessionVariables.GTK_USE_PORTAL = "1";
  systemd.user.sessionVariables.GTK_USE_PORTAL = "1";
  home.packages = [ backend chooser fileManager ];
  systemd.user.packages = [ backend ];
  # Browser "Show in folder" often uses this D-Bus API, not xdg-open.
  systemd.user.services.dojo-yazi-filemanager = {
    Unit = {
      Description = "Yazi FileManager1 folder/reveal routing";
      After = [ "graphical-session.target" ];
      PartOf = [ "graphical-session.target" ];
    };
    Service = {
      Type = "dbus";
      BusName = "org.freedesktop.FileManager1";
      ExecStart = "${fileManager}/bin/dojo-yazi-filemanager";
    };
    Install.WantedBy = [ "graphical-session.target" ];
  };
  # Explicit registration also works on generic Linux hosts whose portal
  # daemon does not discover portal descriptors in the Nix profile.
  xdg.dataFile = portalServiceFiles // {
    "dbus-1/services/org.freedesktop.FileManager1.service".text = ''
      [D-BUS Service]
      Name=org.freedesktop.FileManager1
      Exec=${fileManager}/bin/dojo-yazi-filemanager
      SystemdService=dojo-yazi-filemanager.service
    '';
    "xdg-desktop-portal/portals/termfilechooser.portal".source =
      "${backend}/share/xdg-desktop-portal/portals/termfilechooser.portal";
    "dbus-1/services/org.freedesktop.impl.portal.desktop.termfilechooser.service".source =
      "${backend}/share/dbus-1/services/org.freedesktop.impl.portal.desktop.termfilechooser.service";
  };
  xdg.configFile."xdg-desktop-portal-termfilechooser/config".text = ''
    [filechooser]
    cmd=${chooser}/bin/dojo-yazi-chooser
    default_dir=${config.home.homeDirectory}
    create_help_file=0
    open_mode=suggested
    save_mode=suggested
  '';
  xdg.desktopEntries.dojo-yazi = {
    name = "Yazi";
    comment = "Browse folders in Yazi (Kitty)";
    exec = "${config.programs.kitty.package}/bin/kitty --title Yazi ${config.programs.yazi.package}/bin/yazi %f";
    terminal = false;
    categories = [ "System" "FileTools" "FileManager" ];
    mimeType = [ "inode/directory" ];
    startupNotify = false;
  };
  # Change one association in the existing user file, not a generated replacement
  # of mimeapps.list: browser and URL scheme defaults must survive activation.
  home.activation.yaziDirectoryDefault = lib.hm.dag.entryAfter [ "writeBoundary" "linkGeneration" ] ''
    run ${pkgs.xdg-utils}/bin/xdg-mime default dojo-yazi.desktop inode/directory
  '';
}
