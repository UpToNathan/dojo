#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
unit="$config_home/systemd/user/dojo-gamepad.service"
mkdir -p "$(dirname "$unit")"
if [[ -e "$unit" || -L "$unit" ]]; then
    mv -- "$unit" "$unit.backup-$(date +%Y%m%d%H%M%S)-$$"
fi
python=$(command -v python3)
cat > "$unit" <<EOF
[Unit]
Description=DualSense controls for Dojo
After=graphical-session.target
PartOf=graphical-session.target
ConditionEnvironment=WAYLAND_DISPLAY

[Service]
ExecStart=$python $repo_root/scripts/dojo-gamepad.py
Restart=on-failure
RestartSec=3
TimeoutStopSec=10
KillMode=mixed

[Install]
WantedBy=graphical-session.target
EOF
systemctl --user daemon-reload
systemctl --user enable --now dojo-gamepad.service
