{ pkgs, lib, ... }:
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
  t3Autostart = pkgs.writeShellScript "t3-server-autostart" ''
    settings=${t3Home}/userdata/desktop-settings.json
    ${disableT3LocalEnvironment}
    if ! ${pkgs.jq}/bin/jq -e '.localEnvironmentEnabled == false' "$settings" >/dev/null 2>&1; then
      echo "T3 Code local environment is still enabled; refusing to start a second server against shared ~/.t3 data. Close T3 and switch Home Manager to apply the remote-only setting, or disable Local environment in T3 Settings → Connections." >&2
      exit 1
    fi
    exec ${t3}/bin/t3 start --no-browser --base-dir ${t3Home}
  '';
  disableT3LocalEnvironment = pkgs.writeShellScript "t3-disable-local-environment" ''
    set -eu
    settings=${t3Home}/userdata/desktop-settings.json
    if ${pkgs.procps}/bin/pgrep -u "$UID" -x t3code >/dev/null 2>&1; then
      echo "T3 Code is running; leaving desktop settings unchanged. Close T3 and switch Home Manager again to enable standalone server autostart safely."
      exit 0
    fi

    ${pkgs.coreutils}/bin/mkdir -p "$(dirname "$settings")"
    lock="''${settings}.dojo-lock"
    if ! ${pkgs.coreutils}/bin/mkdir "$lock" 2>/dev/null; then
      echo "T3 settings are being updated; leaving them unchanged."
      exit 0
    fi

    temp="''${settings}.dojo-tmp-$$"
    cleanup() {
      ${pkgs.coreutils}/bin/rm -f -- "$temp"
      ${pkgs.coreutils}/bin/rmdir "$lock" 2>/dev/null || true
    }
    trap cleanup EXIT

    if [ -e "$settings" ] && ${pkgs.jq}/bin/jq -e '.localEnvironmentEnabled == false' "$settings" >/dev/null 2>&1; then
      exit 0
    fi
    if [ -e "$settings" ] && ! ${pkgs.jq}/bin/jq -e 'type == "object"' "$settings" >/dev/null 2>&1; then
      echo "T3 settings are not valid JSON; leaving them unchanged."
      exit 0
    fi

    if [ -e "$settings" ]; then
      backup="''${settings}.backup-$(${pkgs.coreutils}/bin/date +%Y%m%d%H%M%S)"
      ${pkgs.coreutils}/bin/cp -a -- "$settings" "$backup"
      ${pkgs.coreutils}/bin/cp --attributes-only --preserve=all -- "$settings" "$temp"
      ${pkgs.jq}/bin/jq '.localEnvironmentEnabled = false' "$settings" > "$temp"
    else
      printf '{"localEnvironmentEnabled":false}\n' > "$temp"
      chmod 600 "$temp"
    fi
    ${pkgs.coreutils}/bin/mv -fT -- "$temp" "$settings"
  '';
  prepareT3Client = pkgs.writeShellScriptBin "t3-prepare-client" ''
    exec ${disableT3LocalEnvironment}
  '';
in
{
  home.packages = [ t3 prepareT3Client ];

  # Keep the existing T3 profiles, but stop the desktop from opening a second
  # server against the standalone process. The migration runs only while T3 is
  # closed; autostart separately refuses to run until the setting is false.
  home.activation.t3RemoteOnly = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    ${disableT3LocalEnvironment}
  '';

  # T3's headless `serve` command has no live-server preflight. `start` does.
  # The activation above turns off the desktop's own local backend before the
  # next login, so this process remains the sole owner of shared ~/.t3 state.
  xdg.configFile."autostart/t3-server.desktop".text = ''
    [Desktop Entry]
    Name=T3 Code Server
    Comment=Start the T3 Code backend for desktop and remote clients
    Exec=${t3Autostart}
    TryExec=${t3}/bin/t3
    Terminal=false
    Type=Application
    StartupNotify=false
    X-GNOME-Autostart-enabled=true
  '';
}
