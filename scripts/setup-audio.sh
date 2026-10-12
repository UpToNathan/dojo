#!/usr/bin/env bash
set -euo pipefail
# ArchWiki: https://wiki.archlinux.org/title/PipeWire
sudo pacman -Syu --needed pipewire pipewire-audio pipewire-alsa pipewire-pulse wireplumber alsa-utils
systemctl --user enable --now pipewire.socket pipewire-pulse.socket wireplumber.service
pactl info
wpctl status
