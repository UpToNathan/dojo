{ config, lib, pkgs, ... }:
let
  cfg = config.programs.dojoQt6Annotation;
  annotation = pkgs.stdenv.mkDerivation {
    pname = "dojo-qt6-annotation";
    version = "6.11.2";
    src = ./tools/accessibility/qt6-annotation;
    nativeBuildInputs = [ pkgs.cmake pkgs.pkg-config pkgs.wayland-scanner pkgs.qt6.wrapQtAppsHook ];
    buildInputs = [ pkgs.qt6.qtbase pkgs.wayland ];
    dontWrapQtApps = true;
  };
  launcher = pkgs.writeShellScriptBin "dojo-with-qt6-annotation" ''
    plugin_root=${cfg.package}/lib/dojo/qt-6.11.2/plugins
    case ":''${QT_PLUGIN_PATH:-}:" in
      *":$plugin_root:"*) ;;
      *) export QT_PLUGIN_PATH="$plugin_root''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}" ;;
    esac
    plugin_key=dojo-qt6-annotation-6.11.2
    case ",''${QT_QPA_GENERIC_PLUGINS:-}," in
      *",$plugin_key,"*) ;;
      *) export QT_QPA_GENERIC_PLUGINS="''${QT_QPA_GENERIC_PLUGINS:+$QT_QPA_GENERIC_PLUGINS,}$plugin_key" ;;
    esac
    exec "$@"
  '';
in {
  options.programs.dojoQt6Annotation = {
    enable = lib.mkEnableOption "automatic Qt 6.11.2 native accessibility annotations";
    package = lib.mkOption {
      type = lib.types.package;
      default = annotation;
      description = "Plugin built against the pinned Qt 6.11.2 private ABI.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [{
      assertion = pkgs.qt6.qtbase.version == "6.11.2";
      message = "Dojo's Qt annotation plugin pins the Qt 6.11.2 private ABI; review and rebuild it for another Qt.";
    }];
    home.packages = [ cfg.package launcher ];
    home.sessionVariablesExtra = ''
      case ":''${QT_PLUGIN_PATH:-}:" in
        *":${cfg.package}/lib/dojo/qt-6.11.2/plugins:"*) ;;
        *) export QT_PLUGIN_PATH="${cfg.package}/lib/dojo/qt-6.11.2/plugins''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}" ;;
      esac
      case ",''${QT_QPA_GENERIC_PLUGINS:-}," in
        *",dojo-qt6-annotation-6.11.2,"*) ;;
        *) export QT_QPA_GENERIC_PLUGINS="''${QT_QPA_GENERIC_PLUGINS:+$QT_QPA_GENERIC_PLUGINS,}dojo-qt6-annotation-6.11.2" ;;
      esac
    '';
    xdg.configFile."environment.d/65-dojo-qt6-annotation.conf".text = ''
      QT_PLUGIN_PATH=${cfg.package}/lib/dojo/qt-6.11.2/plugins''${QT_PLUGIN_PATH:+:}''${QT_PLUGIN_PATH}
      QT_QPA_GENERIC_PLUGINS=dojo-qt6-annotation-6.11.2''${QT_QPA_GENERIC_PLUGINS:+,}''${QT_QPA_GENERIC_PLUGINS}
    '';
  };
}
