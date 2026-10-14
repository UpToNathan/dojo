#!/usr/bin/env bash
set -euo pipefail

if ! command -v pacman >/dev/null; then
    printf 'This setup command requires Arch Linux.\n' >&2
    exit 1
fi

printf 'Install iPhone USB pairing support and load the USB Ethernet driver.\n'
new_usb_rules=false
if ! pacman -Qq usbmuxd >/dev/null 2>&1; then
    new_usb_rules=true
fi
sudo pacman -S --needed usbmuxd libimobiledevice
sudo udevadm control --reload-rules
sudo modprobe ipheth
if $new_usb_rules; then
    sudo udevadm trigger --action=add --subsystem-match=usb --attr-match=idVendor=05ac
    sudo udevadm settle
fi
sudo systemctl start usbmuxd.service

printf '\nUnlock your iPhone, enable Personal Hotspot, and tap Trust if prompted.\n'
if ! idevicepair validate; then
    idevicepair pair
fi
printf '\nNetworkManager devices:\n'
nmcli -f DEVICE,TYPE,STATE,CONNECTION device
printf '\nIf USB Ethernet is not listed, reconnect the USB cable with Personal Hotspot enabled.\n'
