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

# Sources that name the account carry @USER@; the installed copy names whoever runs this script.
target_user=${SUDO_USER:-$(id -un)}
rendered_dir=$(mktemp -d)
trap 'rm -rf -- "$rendered_dir"' EXIT

theme_only=false
if [[ ${1:-} == --chromium-theme-only ]]; then theme_only=true; shift; fi
[[ $# -eq 0 ]] || { echo "Usage: $0 [--chromium-theme-only]" >&2; exit 2; }

# Everything root reads or runs is a root-owned copy, never a symlink into this user-writable
# checkout: a link would let any process of the checkout's owner rewrite root's PAM stack, units
# and scripts. Re-run this installer after editing a file here.
# The first two entries are the Chromium theme integration, which is all --chromium-theme-only installs.
config_copies=(
    'bin/dojo-chromium-theme-policy|/usr/local/bin/dojo-chromium-theme-policy|0755'
    'sudoers.d/dojo-chromium-theme-policy|/etc/sudoers.d/dojo-chromium-theme-policy|0440'
    '1password/custom_allowed_browsers|/etc/1password/custom_allowed_browsers|0644'
    'nix/nix.conf|/etc/nix/nix.conf|0644'
    'pam.d/system-local-login|/etc/pam.d/system-local-login|0644'
    'systemd/system/update-nix-gl.service|/etc/systemd/system/update-nix-gl.service|0644'
    'tmpfiles.d/nix-pam-unix-chkpwd.conf|/etc/tmpfiles.d/nix-pam-unix-chkpwd.conf|0644'
    'tmpfiles.d/opengl-driver.conf|/etc/tmpfiles.d/opengl-driver.conf|0644'
    'udev/rules.d/90-wluma-backlight.rules|/etc/udev/rules.d/90-wluma-backlight.rules|0644'
    'bin/update-nix-gl|/usr/local/bin/update-nix-gl|0755'
    'pam.d/dojo-lock-fingerprint|/etc/pam.d/dojo-lock-fingerprint|0644'
    'pam.d/dojo-lock-password|/etc/pam.d/dojo-lock-password|0644'
    'systemd/logind.conf.d/90-dojo-power-key.conf|/etc/systemd/logind.conf.d/90-dojo-power-key.conf|0644'
    'pam.d/dojo-login|/etc/pam.d/dojo-login|0644'
    'pam.d/dojo-login-fingerprint|/etc/pam.d/dojo-login-fingerprint|0644'
    'pam.d/dojo-greeter|/etc/pam.d/dojo-greeter|0644'
    'sysusers.d/dojo-greeter.conf|/etc/sysusers.d/dojo-greeter.conf|0644'
    'tmpfiles.d/dojo-dm.conf|/etc/tmpfiles.d/dojo-dm.conf|0644'
    'systemd/system/dojo-dm.service|/etc/systemd/system/dojo-dm.service|0644'
    'systemd/system/getty@tty2.service.d/dojo-rescue.conf|/etc/systemd/system/getty@tty2.service.d/dojo-rescue.conf|0644'
    'wayland-sessions/niri-uwsm.desktop|/usr/share/wayland-sessions/niri-uwsm.desktop|0644'
)

# Battery settings apply only where there is a battery. Root-owned copies, not symlinks: udev
# rules run programs as root, so they must not follow a user-writable checkout.
laptop=false
if compgen -G '/sys/class/power_supply/BAT*' >/dev/null; then laptop=true; fi
if $laptop && ! $theme_only; then
    config_copies+=(
        'laptop/sysctl.d/90-dojo-power.conf|/etc/sysctl.d/90-dojo-power.conf|0644'
        'laptop/NetworkManager/conf.d/90-dojo-wifi-powersave.conf|/etc/NetworkManager/conf.d/90-dojo-wifi-powersave.conf|0644'
        'laptop/udev/rules.d/90-dojo-power.rules|/etc/udev/rules.d/90-dojo-power.rules|0644'
        'laptop/tmpfiles.d/90-dojo-battery.conf|/etc/tmpfiles.d/90-dojo-battery.conf|0644'
        'laptop/systemd/logind.conf.d/90-dojo-lid.conf|/etc/systemd/logind.conf.d/90-dojo-lid.conf|0644'
        'laptop/systemd/sleep.conf.d/90-dojo-hibernate.conf|/etc/systemd/sleep.conf.d/90-dojo-hibernate.conf|0644'
    )
fi

if $theme_only; then config_copies=("${config_copies[@]:0:2}"); fi
for config_copy in "${config_copies[@]}"; do
    source_relative=${config_copy%%|*}
    remainder=${config_copy#*|}
    target_file=${remainder%%|*}
    mode=${remainder##*|}
    source_file="$repo_root/system-config/$source_relative"
    if grep -q '@USER@' -- "$source_file"; then
        sed "s/@USER@/$target_user/g" -- "$source_file" > "$rendered_dir/$(basename "$source_file")"
        source_file="$rendered_dir/$(basename "$source_file")"
    fi

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

if $theme_only; then
    printf 'Installed Chromium theme integration; active policy preserved.\n'
fi

if ! $theme_only; then
    printf 'The power-key setting in logind takes effect at the next boot.\n'
    "$repo_root/system-config/bin/configure-firewalld"
    # The greeter user and its state directory must exist before dojo-dm.service is enabled.
    as_root systemd-sysusers /etc/sysusers.d/dojo-greeter.conf
    as_root systemd-tmpfiles --create /etc/tmpfiles.d/dojo-dm.conf
    as_root systemctl daemon-reload
    as_root udevadm control --reload-rules
    if $laptop; then
        as_root sysctl --system >/dev/null
        as_root systemd-tmpfiles --create /etc/tmpfiles.d/90-dojo-battery.conf
        as_root udevadm trigger --subsystem-match=power_supply --action=change
        if as_root systemctl is-active --quiet NetworkManager.service; then
            as_root nmcli general reload conf
        fi
        printf 'Battery settings installed. The lid action in logind takes effect at the next boot.\n'
    fi
fi
