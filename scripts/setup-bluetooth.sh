#!/usr/bin/env bash
set -euo pipefail
if ! command -v pacman >/dev/null; then
    printf 'This setup command is for Arch Linux. Install and enable BlueZ with your system package manager.\n' >&2
    exit 1
fi
printf 'Install BlueZ and its Bluetooth tools, then enable the system Bluetooth service.\n'
printf 'Arch setup reference: https://wiki.archlinux.org/title/Bluetooth\n'
sudo pacman -S --needed bluez bluez-utils
sudo systemctl enable --now bluetooth.service
systemctl is-active --quiet bluetooth.service
busctl --system tree org.bluez
bluetoothctl list
rfkill list bluetooth
printf '\nBlueZ is running. Open the Bluetooth dock in Dojo to turn the adapter on and pair a device.\n'
printf 'If no controller is listed, check the adapter driver and rfkill state before pairing.\n'
