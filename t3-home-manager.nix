{ pkgs, ... }:
let
  t3AppDir = "/home/nathan/.local/opt/t3code";
  t3Home = "/home/nathan/.t3";
  t3 = pkgs.writeShellScriptBin "t3" ''
    app_dir=${t3AppDir}
    if [ ! -x "$app_dir/t3code" ]; then
      echo "T3 Code executable is missing at $app_dir/t3code" >&2
      exit 127
    fi

    export T3CODE_CLI_PATH="$0"
    export T3CODE_HOME="''${T3CODE_HOME:-${t3Home}}"
    export ELECTRON_RUN_AS_NODE=1
    export LD_LIBRARY_PATH="$app_dir/usr/lib''${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
    exec "$app_dir/t3code" "$app_dir/resources/app.asar/apps/server/dist/bin.mjs" "$@"
  '';

in
{
  home.packages = [ t3 ];
}
