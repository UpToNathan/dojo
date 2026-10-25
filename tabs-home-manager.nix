# The Dojo Tabs extension (extensions/dojo-tabs) and the native-messaging host it reports to.
# The extension tells Dojo which tabs each browser window holds, so a whole window can be sent to
# another machine (docs/mesh.md). Helium loads it from the stable path below, next to Browser Control's
# (see the `helium` wrapper in home-manager.nix); a running Helium must exit normally and relaunch to
# pick it up.
{ config, ... }:
let
  home = config.home.homeDirectory;
  # The id the extension's manifest key makes; dojo-mesh's own tests check it against that key.
  extensionId = "ifbpcpbmclepnkapoahcikkldpjnkgcf";
in
{
  home.file.".local/share/dojo/tabs-extension".source = ./extensions/dojo-tabs;

  # Chromium starts a native host by the absolute path in its manifest, with the caller's origin as
  # the only argument, so the host is a one-line launcher for dojo-mesh.
  home.file.".local/share/dojo/bin/dojo-tabs-host" = {
    executable = true;
    text = ''
      #!/bin/sh
      exec "''${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh" tabs-host "$@"
    '';
  };

  xdg.configFile."net.imput.helium/NativeMessagingHosts/net.dojo.tabs.json".text = builtins.toJSON {
    name = "net.dojo.tabs";
    description = "Reports the browser's windows and tabs to Dojo";
    path = "${home}/.local/share/dojo/bin/dojo-tabs-host";
    type = "stdio";
    allowed_origins = [ "chrome-extension://${extensionId}/" ];
  };
}
