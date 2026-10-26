# Installer for the T3 Code app

Type: task
Status: resolved

Write `scripts/install-t3code.py` with a test, call it from `dojo-update` and `bootstrap-host`, and document it in `docs/t3-server.md`. Spec: [spec.md](../spec.md).

## Answer

`scripts/install-t3code.py` (tests in `scripts/test-install-t3code.py`) installs from the GitHub release's x86_64 or arm64 AppImage, verified against GitHub's digest and `SHA256SUMS`, extracted with `--appimage-extract` and swapped in by rename. `dojo-update`'s `t3code` group and `bootstrap-host user` call it. Checked against the real `v0.1.3-nightly.20261026.3717` release into a scratch prefix. The existing `~/.local/opt/t3code` (0.0.46) and Jupiter were not touched.
