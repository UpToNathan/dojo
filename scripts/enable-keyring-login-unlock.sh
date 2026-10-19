#!/usr/bin/env bash
# Makes the GNOME keyring unlock with the Dojo login screen's password login, so
# Dojo's stored credentials are available without a dialog.
#
# The dojo-login PAM stack already lists pam_gnome_keyring.so, marked optional
# with a leading "-", so a missing module is skipped silently. The Home Manager daemon
# lives in the Nix store, and a Nix-built PAM module must not be loaded into
# the host's login process, so the Arch package supplies the module. The module
# only ever unlocks a keyring named "login", so an existing default keyring
# is renamed to that. Its password must already equal the login password.
set -euo pipefail

if (( EUID == 0 )); then
    printf 'Run this as your own user; it uses sudo only to install the package.\n' >&2
    exit 1
fi

module=/usr/lib/security/pam_gnome_keyring.so
keyrings=${XDG_DATA_HOME:-"$HOME/.local/share"}/keyrings

if [[ ! -e $module ]]; then
    sudo pacman -S --needed gnome-keyring
fi
[[ -e $module ]] || { printf '%s is still missing after installing gnome-keyring.\n' "$module" >&2; exit 1; }
grep -q '^-\?session.*pam_gnome_keyring.so.*auto_start' /etc/pam.d/dojo-login ||
    { printf '/etc/pam.d/dojo-login does not start the keyring on login; run scripts/install-system-config.sh first.\n' >&2; exit 1; }

default_name=$(cat "$keyrings/default" 2>/dev/null || true)
if [[ -e $keyrings/login.keyring ]]; then
    printf 'login.keyring already exists; leaving the keyrings as they are.\n'
elif [[ -n $default_name && -e $keyrings/$default_name.keyring ]]; then
    backup=$keyrings/backup-$(date +%Y%m%d%H%M%S)
    systemctl --user stop gnome-keyring.service 2>/dev/null || true
    pkill -u "$USER" -f gnome-keyring-daemon 2>/dev/null || true
    mkdir -m 0700 "$backup"
    cp -p -- "$keyrings"/*.keyring "$keyrings/default" "$backup"/
    mv -- "$keyrings/$default_name.keyring" "$keyrings/login.keyring"
    printf 'login\n' > "$keyrings/default"
    printf 'Renamed the "%s" keyring to login. A copy is in %s.\n' "$default_name" "$backup"
    systemctl --user start gnome-keyring.service 2>/dev/null || true
else
    printf 'No default keyring exists yet; the first login will create "login" with your password.\n'
fi

printf 'Log out and back in with your password (not autologin) to finish. Then check with:\n'
printf "  gdbus call --session --dest org.freedesktop.secrets --object-path /org/freedesktop/secrets/aliases/default \\\\\n"
printf "    --method org.freedesktop.DBus.Properties.Get org.freedesktop.Secret.Collection Locked\n"
