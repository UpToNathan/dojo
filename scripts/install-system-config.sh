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

config_copies=(
    'bin/dojo-chromium-theme-policy|/usr/local/bin/dojo-chromium-theme-policy|0755'
    'sudoers.d/dojo-chromium-theme-policy|/etc/sudoers.d/dojo-chromium-theme-policy|0440'
)

for config_copy in "${config_copies[@]}"; do
    source_relative=${config_copy%%|*}
    remainder=${config_copy#*|}
    target_file=${remainder%%|*}
    mode=${remainder##*|}
    source_file="$repo_root/system-config/$source_relative"

    if [[ $target_file == /etc/sudoers.d/* ]]; then
        as_root visudo -cf "$source_file"
    fi
    if [[ -f $target_file && ! -L $target_file && $(stat -c '%a' "$target_file") == "$mode" ]] && cmp -s -- "$source_file" "$target_file"; then
        continue
    fi

    as_root install -d "$(dirname "$target_file")"
    if [[ -e $target_file || -L $target_file ]]; then
        backup="$target_file.backup-$(date +%Y%m%d%H%M%S)-$$"
        as_root mv -- "$target_file" "$backup"
        printf 'Backed up existing file to %s\n' "$backup"
    fi
    as_root install -o root -g root -m "$mode" -- "$source_file" "$target_file"
    printf 'Installed %s (copy)\n' "$target_file"
done

for policy_dir in /etc/chromium /etc/chromium/policies /etc/chromium/policies/managed; do
    if [[ -L $policy_dir || ( -e $policy_dir && ! -d $policy_dir ) ]]; then
        printf 'Refusing unsafe Chromium policy directory %s\n' "$policy_dir" >&2
        exit 1
    fi
    if [[ ! -e $policy_dir ]]; then
        as_root install -d -m 0755 -o root -g root -- "$policy_dir"
    fi
    if [[ $(stat -c '%U:%G %a' "$policy_dir") != 'root:root 755' ]]; then
        printf 'Chromium policy directory must be root:root mode 0755: %s\n' "$policy_dir" >&2
        exit 1
    fi
done

as_root systemctl daemon-reload
as_root udevadm control --reload-rules
