#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

as_root() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    else
        sudo "$@"
    fi
}

config_links=(
    '1password/custom_allowed_browsers|/etc/1password/custom_allowed_browsers'
    'nix/nix.conf|/etc/nix/nix.conf'
    'pam.d/system-local-login|/etc/pam.d/system-local-login'
    'systemd/system/update-nix-gl.service|/etc/systemd/system/update-nix-gl.service'
    'tmpfiles.d/nix-pam-unix-chkpwd.conf|/etc/tmpfiles.d/nix-pam-unix-chkpwd.conf'
    'tmpfiles.d/opengl-driver.conf|/etc/tmpfiles.d/opengl-driver.conf'
    'udev/rules.d/90-wluma-backlight.rules|/etc/udev/rules.d/90-wluma-backlight.rules'
    'bin/update-nix-gl|/usr/local/bin/update-nix-gl'
)

for config_link in "${config_links[@]}"; do
    source_relative=${config_link%%|*}
    target_file=${config_link#*|}
    source_file="$repo_root/system-config/$source_relative"
    if [[ -L $target_file && $(readlink -f "$target_file") == "$source_file" ]]; then
        continue
    fi

    as_root install -d "$(dirname "$target_file")"
    if [[ -e $target_file || -L $target_file ]]; then
        backup="$target_file.backup-$(date +%Y%m%d%H%M%S)-$$"
        as_root mv -- "$target_file" "$backup"
        printf 'Backed up existing file to %s\n' "$backup"
    fi
    as_root ln -s -- "$source_file" "$target_file"
    printf 'Installed %s -> %s\n' "$target_file" "$source_file"
done

as_root systemctl daemon-reload
as_root udevadm control --reload-rules
