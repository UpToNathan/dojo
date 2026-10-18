{ config, lib, ... }:
{
  # The service keeps the config it started with; a deployment that changed the idle plan
  # reaches the running daemon here. It is not running on the first deployment, which is fine.
  home.activation.reloadStasis = lib.hm.dag.entryAfter [ "reloadSystemd" ] ''
    ${config.services.stasis.package}/bin/stasis reload >/dev/null 2>&1 || true
  '';
}
