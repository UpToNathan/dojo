{ config, lib, pkgs, ... }:
let
  cfg = config.programs.dojoQt5Annotation;
  backend = assert lib.assertMsg
    (builtins.hashFile "sha256" ./tools/accessibility/qt5-annotation/qtwayland-atspi-connection.patch
      == "b8343b4bc11f524e295208259e9ceaf19daa0cb13ea722ffa4327df60b1b9179")
    "Qt5 connection accessor patch differs from its provenance";
    pkgs.qt5.qtwayland.overrideAttrs (old: {
    patches = (old.patches or []) ++ [ ./tools/accessibility/qt5-annotation/qtwayland-atspi-connection.patch ];
  });
  annotation = pkgs.stdenv.mkDerivation {
    pname = "dojo-qt5-annotation";
    version = "5.15.19";
    src = ./tools/accessibility;
    postUnpack = ''sourceRoot="$sourceRoot/qt5-annotation"'';
    nativeBuildInputs = [ pkgs.cmake pkgs.pkg-config pkgs.wayland-scanner pkgs.qt5.wrapQtAppsHook ];
    buildInputs = [ pkgs.qt5.qtbase backend pkgs.wayland ];
    cmakeFlags = [ "-DDOJO_QT5_BACKEND_PLUGINS=${backend.bin}/lib/qt-5.15.19/plugins" ];
    dontWrapQtApps = true;
  };
  launcher = pkgs.writeShellScriptBin "dojo-with-qt5-annotation" ''
    export QT_PLUGIN_PATH="${cfg.package}/lib/dojo/qt-5.15.19/plugins''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}"
    export QT_QPA_PLATFORM_PLUGIN_PATH="''${QT_QPA_PLATFORM_PLUGIN_PATH:-${cfg.package}/lib/dojo/qt-5.15.19/qpa-gate}"
    case ",''${QT_QPA_GENERIC_PLUGINS:-}," in
      *",dojo-qt5-annotation-5.15.19,"*) ;;
      *) export QT_QPA_GENERIC_PLUGINS="''${QT_QPA_GENERIC_PLUGINS:+$QT_QPA_GENERIC_PLUGINS,}dojo-qt5-annotation-5.15.19" ;;
    esac
    exec "$@"
  '';
in {
  options.programs.dojoQt5Annotation = {
    enable = lib.mkEnableOption "the pinned patched Qt 5.15.19 annotation launcher";
    package = lib.mkOption {
      type = lib.types.package;
      default = annotation;
      description = "Annotation plugin for the patched Qt 5.15.19 Wayland backend.";
    };
    backend = lib.mkOption {
      type = lib.types.package;
      default = backend;
      readOnly = true;
      description = "QtWayland rebuilt with the authoritative AT-SPI connection accessor.";
    };
  };
  config = lib.mkIf cfg.enable {
    assertions = [{
      assertion = pkgs.qt5.qtbase.version == "5.15.19" && pkgs.qt5.qtwayland.version == "5.15.19";
      message = "Dojo Qt5 annotations require the pinned Qt 5.15.19 runtime.";
    }];
    home.packages = [ cfg.package launcher ];
    home.sessionVariablesExtra = ''
      case ":''${QT_PLUGIN_PATH:-}:" in
        *":${cfg.package}/lib/dojo/qt-5.15.19/plugins:"*) ;;
        *) export QT_PLUGIN_PATH="${cfg.package}/lib/dojo/qt-5.15.19/plugins''${QT_PLUGIN_PATH:+:$QT_PLUGIN_PATH}" ;;
      esac
      case ",''${QT_QPA_GENERIC_PLUGINS:-}," in
        *",dojo-qt5-annotation-5.15.19,"*) ;;
        *) export QT_QPA_GENERIC_PLUGINS="''${QT_QPA_GENERIC_PLUGINS:+$QT_QPA_GENERIC_PLUGINS,}dojo-qt5-annotation-5.15.19" ;;
      esac
      export QT_QPA_PLATFORM_PLUGIN_PATH="''${QT_QPA_PLATFORM_PLUGIN_PATH:-${cfg.package}/lib/dojo/qt-5.15.19/qpa-gate}"
    '';
    xdg.configFile."environment.d/66-dojo-qt5-annotation.conf".text = ''
      QT_PLUGIN_PATH=${cfg.package}/lib/dojo/qt-5.15.19/plugins''${QT_PLUGIN_PATH:+:}''${QT_PLUGIN_PATH}
      QT_QPA_GENERIC_PLUGINS=dojo-qt5-annotation-5.15.19''${QT_QPA_GENERIC_PLUGINS:+,}''${QT_QPA_GENERIC_PLUGINS}
      QT_QPA_PLATFORM_PLUGIN_PATH=''${QT_QPA_PLATFORM_PLUGIN_PATH:-${cfg.package}/lib/dojo/qt-5.15.19/qpa-gate}
    '';
  };
}
