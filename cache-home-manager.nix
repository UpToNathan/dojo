# Serves this host's Nix store to the tailnet as a signed binary cache (harmonia) when the host
# declares a nix-cache capability in its mesh configuration (docs/mesh.md). Other hosts find it by
# asking the mesh (scripts/update-nix-cache.sh), so a package compiled on one machine is
# downloaded, not compiled, on the other.
{ config, lib, pkgs, ... }:
let
  meshFile = "${config.home.homeDirectory}/.config/dojo/mesh.toml";
  mesh = if builtins.pathExists meshFile then builtins.fromTOML (builtins.readFile meshFile) else { };
  declared = lib.attrByPath [ "capability" "nix-cache" ] null mesh;
  hostname = lib.strings.trim (builtins.readFile /etc/hostname);
  cache = {
    port = 5000;
    keyName = "${hostname}-dojo-1";
  } // lib.optionalAttrs (declared != null) (
    lib.optionalAttrs (declared ? port) { port = declared.port; }
    // lib.optionalAttrs (declared ? "key-name") { keyName = declared."key-name"; }
  );
  serve = pkgs.writeShellScript "dojo-nix-cache" ''
    set -euo pipefail
    state="''${XDG_CONFIG_HOME:-$HOME/.config}/dojo/nix-cache"
    runtime="''${XDG_RUNTIME_DIR:?}/dojo-nix-cache"
    install -d -m 700 "$state" "$runtime"
    # The signing key never leaves this host and is not tracked. Its public half belongs in
    # extra-trusted-public-keys of system-config/nix/nix.conf; a new key needs that line updated.
    if [[ ! -f $state/secret-key ]]; then
      ${pkgs.nix}/bin/nix-store --generate-binary-cache-key ${cache.keyName} "$state/secret-key" "$state/public-key"
    fi
    # Listen on the Tailscale address only, never the LAN.
    address=$(/usr/bin/tailscale ip -4 | head -n 1)
    [[ -n $address ]] || { echo "Tailscale has no IPv4 address yet" >&2; exit 1; }
    cat > "$runtime/harmonia.toml" <<CONFIG
    bind = "$address:${toString cache.port}"
    sign_key_paths = ["$state/secret-key"]
    enable_compression = true
    priority = 50
    CONFIG
    CONFIG_FILE="$runtime/harmonia.toml" exec ${pkgs.harmonia}/bin/harmonia-cache
  '';
in
{
  config = lib.mkIf (declared != null) {
    systemd.user.services.dojo-nix-cache = {
      Unit.Description = "Serve the Nix store to the tailnet (harmonia)";
      Service = {
        ExecStart = "${serve}";
        Restart = "on-failure";
        RestartSec = 10;
        NoNewPrivileges = true;
      };
      Install.WantedBy = [ "default.target" ];
    };
  };
}
