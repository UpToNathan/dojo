# Scoped Home Manager artifact build without the desktop, activation or deployment.
# nix-build --no-out-link scripts/check-yazi-module.nix \
#   --arg pkgs '(import /path/to/pinned/nixpkgs {})' \
#   --arg homeManagerPath /path/to/pinned/home-manager
{ pkgs, homeManagerPath, homeDirectory ? "/home/test", username ? "test" }:
let
  hm = import (homeManagerPath + "/modules") {
    inherit pkgs;
    configuration = {
      imports = [ ../yazi-home-manager.nix ];
      home.username = username;
      home.homeDirectory = homeDirectory;
      home.stateVersion = "26.05";
      programs.yazi.enable = true;
      programs.kitty.enable = true;
    };
  };
  packages = builtins.filter
    (p: builtins.match ".*dojo-yazi.*" (p.name or "") != null)
    hm.config.home.packages;
in
assert builtins.all (a: a.assertion) hm.config.assertions;
pkgs.linkFarm "dojo-yazi-module-check" (
  map (p: { name = p.name; path = p; }) packages
  ++ [
    { name = "portal-routing"; path = hm.config.xdg.configFile."xdg-desktop-portal/portals.conf".source; }
    { name = "dojo-portal"; path = hm.config.xdg.dataFile."xdg-desktop-portal/portals/dojo.portal".source; }
    { name = "config"; path = hm.config.xdg.configFile."xdg-desktop-portal-termfilechooser/config".source; }
    { name = "backend"; path = pkgs.xdg-desktop-portal-termfilechooser; }
    { name = "dbus-filemanager"; path = hm.config.xdg.dataFile."dbus-1/services/org.freedesktop.FileManager1.service".source; }
    { name = "systemd-filemanager"; path = hm.config.xdg.configFile."systemd/user/dojo-yazi-filemanager.service".source; }
  ]
)
