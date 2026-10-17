{ config, lib, pkgs, ... }:
let
  theme = "${config.xdg.stateHome}/dojo/themes/current";
  root = config.dojo.root;
  qtSettings = version: {
    Appearance = {
      style = "Fusion";
      custom_palette = true;
      color_scheme_path = "${theme}/qt${version}.conf";
      standard_dialogs = "xdgdesktopportal";
    };
  };
  host = pkgs.writeShellScript "dojo-zen-host" ''
    exec ${root}/target/debug/dojo-zen-host
  '';
  manifest = {
    name = "org.dojo.theme";
    description = "Dojo live theme bridge";
    path = toString host;
    type = "stdio";
    allowed_extensions = [ "dojo-theme@local" ];
  };
in {
  options.dojo.themeIntegrations.enable = lib.mkEnableOption "Dojo's generated desktop theme integrations";
  config = lib.mkIf config.dojo.themeIntegrations.enable {
  home.packages = [
    (pkgs.writeShellScriptBin "dojo-theme" ''
      exec ${root}/target/debug/dojo-theme "$@"
    '')
    (pkgs.writeShellScriptBin "dojo-qt5" ''
      export QT_QPA_PLATFORMTHEME=qt5ct
      export QT_STYLE_OVERRIDE=Fusion
      exec "$@"
    '')
    (pkgs.writeShellScriptBin "dojo-qt6" ''
      export QT_QPA_PLATFORMTHEME=qt6ct
      export QT_STYLE_OVERRIDE=Fusion
      exec "$@"
    '')
  ];
  programs.kitty.extraConfig = ''
    include ${theme}/kitty.conf
  '';
  xdg.configFile."ghostty/config".text = lib.mkAfter ''
    config-file = ?${theme}/ghostty.conf
  '';
  xdg.configFile."dojo/nvim-presets.json".text = builtins.toJSON {
    tokyonight = "${pkgs.vimPlugins.tokyonight-nvim}";
    catppuccin = "${pkgs.vimPlugins.catppuccin-nvim}";
  };
  programs.yazi = {
    plugins."dojo-theme" = ./yazi-config/dojo-theme.yazi;
    initLua = ''
      require("dojo-theme"):setup()
    '';
    theme.flavor = { dark = "dojo"; light = "dojo"; };
  };
  qt = {
    enable = true;
    platformTheme.name = "qtct";
    style.name = "Fusion";
    qt5ctSettings = qtSettings "5";
    qt6ctSettings = qtSettings "6";
  };
  # Qt's platform selector is singular. Qt6 is the session default; Qt5 apps use
  # the supplied version-specific wrapper instead of an invalid combined value.
  home.sessionVariables.QT_QPA_PLATFORMTHEME = lib.mkForce "qt6ct";
  systemd.user.sessionVariables.QT_QPA_PLATFORMTHEME = lib.mkForce "qt6ct";
  home.file.".mozilla/native-messaging-hosts/org.dojo.theme.json".text = builtins.toJSON manifest;
  xdg.configFile."zen/native-messaging-hosts/org.dojo.theme.json".text = builtins.toJSON manifest;
  xdg.dataFile."dojo/zen-theme".source = ./zen-theme;
  };
}
