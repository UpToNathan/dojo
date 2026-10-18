# 02 — Roll the GPUI and Niri picks out to venus and Jupiter

Type: task
Status: ready-for-human
Blocked by: none
Effort: small

## Brief

Venus runs the new GPUI release. Neither host runs the new Niri build (it needs
`scripts/install-niri-fork.sh` through `scripts/bootstrap-host user`, then a login). Jupiter has not
received the GPUI, Niri, stasis or `dojo-update` changes at all.

1. Push the commits to Jupiter (`GIT_SSH_COMMAND="tailscale ssh" git push nathan@jupiter:Dojo
   master:refs/heads/from-venus`, then `git reset --hard` there) and run `scripts/bootstrap-host user`.
2. Log out and back in on both hosts (or reboot).
3. Check by eye: translucent windows (source-over alpha change `e18de65b75`), the wallpaper span, the
   lock screen, and a monitor unplug and replug on Jupiter (wgpu teardown fix `402675e805`).
4. Confirm with `niri msg version` that the running compositor matches the installed one, and that
   `toggle-touchpad` appears in `niri msg action --help`.

## Acceptance

Both hosts run the same Dojo, GPUI and Niri builds; a monitor replug does not crash the shell.
