{ pkgs, lib, ... }:
let
  browserControl = pkgs.callPackage ./nix/browser-control.nix { };
  skill = "${browserControl}/lib/browser-control/skills/browser-control";
in
{
  home.packages = [ browserControl browserControl.broker ];
  home.sessionVariables.DOJO_BROWSER_CONTROL_BROKER_EXE =
    "${browserControl.broker}/bin/dojo-browser-control-broker";
  home.sessionVariables.DOJO_BROWSER_CONTROL_MCP_EXE =
    "${browserControl}/bin/browser-control-mcp";
  home.file.".local/share/dojo/browser-control/extension".source =
    "${browserControl}/lib/browser-control/extension/dist";
  home.file.".agents/skills/browser-control".source = skill;
  home.file.".claude/skills/browser-control".source = skill;
  home.activation.dojoBrowserControlConfig = lib.hm.dag.entryAfter [ "writeBoundary" ] ''
    ${pkgs.python3}/bin/python3 ${./scripts/install-browser-control-config.py} \
      --command ${browserControl}/bin/browser-control-mcp
  '';
}
