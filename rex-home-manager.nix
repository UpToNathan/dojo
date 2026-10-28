{ ... }:
{
  # A personal Rex server as a systemd user service (journalctl --user -u rex). It runs from a
  # store path, so nothing here starts through devenv. `loginctl enable-linger`, run by
  # scripts/bootstrap-host, keeps it running while logged out.
  services.rex.enable = true;
  # The ssh guard goes through programs.ssh, which would take over the ~/.ssh/config that the
  # mesh roster edits; `rex terminfo setup` adds it to that file where wanted.
  programs.rex.sshGuard = false;
}
