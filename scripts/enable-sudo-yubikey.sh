#!/usr/bin/env bash
# Optional: makes sudo on this machine ask for a touch of a registered security key (a YubiKey or
# any FIDO2 key) as well as the password. Nothing in the mesh requires this; it is for a machine
# whose owner has a key. Three steps, run in order:
#   register   add a key for a user to the root-owned mapping file (touch the key when it blinks)
#   enable     add the pam_u2f line to /etc/pam.d/sudo
#   disable    take that line out again
#
# Recovery if sudo ever refuses you: keep a root shell open while you test `sudo true` in another
# terminal. From a console without one, `su -` (root's password) or a rescue boot, delete the line
# marked "dojo-mesh" from /etc/pam.d/sudo; `disable` does the same. Register a second key as a spare.
# A user with no registered key is not asked for one (nouserok), so this cannot lock out anyone else.
set -euo pipefail

pam_dir=${DOJO_PAM_DIR:-/etc/pam.d}
mapping=${DOJO_U2F_MAPPING:-/etc/dojo-u2f-mappings}
module=${DOJO_PAM_U2F:-/usr/lib/security/pam_u2f.so}
marker='# dojo-mesh: sudo asks for a security key touch'
line="auth       required     pam_u2f.so authfile=$mapping origin=pam://dojo appid=pam://dojo cue nouserok"

usage() {
    echo "Usage: $0 register [--user NAME] | enable | disable" >&2
}

# Files are root-owned copies; a test run (DOJO_NO_SUDO) has no root to give them to.
owner=(-o root -g root)
[[ -z ${DOJO_NO_SUDO:-} ]] || owner=()

as_root() {
    if [[ $EUID -eq 0 || -n ${DOJO_NO_SUDO:-} ]]; then
        "$@"
    else
        sudo "$@"
    fi
}

[[ $# -ge 1 ]] || { usage; exit 2; }
action=$1; shift
target_user=${SUDO_USER:-$(id -un)}
if [[ $action == register && ${1:-} == --user ]]; then
    [[ $# -ge 2 ]] || { usage; exit 2; }
    target_user=$2; shift 2
fi
[[ $# -eq 0 ]] || { usage; exit 2; }

sudo_file=$pam_dir/sudo
case $action in
    register)
        command -v pamu2fcfg >/dev/null || { echo "pamu2fcfg is not installed (Arch: pam-u2f)." >&2; exit 1; }
        entry=$(pamu2fcfg -u "$target_user" -o pam://dojo -i pam://dojo)
        [[ $entry == "$target_user":* ]] || { echo "No key was registered." >&2; exit 1; }
        # Root owns the mapping, so a process of the user cannot add a key of its own.
        as_root install "${owner[@]}" -m 0644 /dev/null "$mapping.new"
        { [[ -f $mapping ]] && cat -- "$mapping"; printf '%s\n' "$entry"; } | as_root tee "$mapping.new" >/dev/null
        as_root mv -- "$mapping.new" "$mapping"
        echo "Registered a key for $target_user in $mapping. Register a second one as a spare, then run: $0 enable"
        ;;
    enable)
        [[ -f $module ]] || { echo "$module is missing (Arch: pam-u2f)." >&2; exit 1; }
        [[ -s $mapping ]] || { echo "No key is registered yet ($mapping): run $0 register first." >&2; exit 1; }
        if grep -qsF -- "$marker" "$sudo_file"; then echo "Already enabled."; exit 0; fi
        grep -qE '^auth' -- "$sudo_file" || { echo "$sudo_file has no auth line to put this before." >&2; exit 1; }
        backup="$sudo_file.backup-$(date +%Y%m%d%H%M%S)-$$"
        as_root cp -p -- "$sudo_file" "$backup"
        printf 'Backed up %s to %s\n' "$sudo_file" "$backup"
        new=$(mktemp)
        trap 'rm -f -- "$new"' EXIT
        awk -v marker="$marker" -v line="$line" '!done && /^auth/ { print marker; print line; done = 1 } { print }' "$sudo_file" > "$new"
        as_root install "${owner[@]}" -m 0644 -- "$new" "$sudo_file"
        echo "Enabled. In ANOTHER terminal run 'sudo true' and touch the key; keep this root shell until it works."
        ;;
    disable)
        grep -qsF -- "$marker" "$sudo_file" || { echo "Not enabled."; exit 0; }
        backup="$sudo_file.backup-$(date +%Y%m%d%H%M%S)-$$"
        as_root cp -p -- "$sudo_file" "$backup"
        new=$(mktemp)
        trap 'rm -f -- "$new"' EXIT
        awk -v marker="$marker" '$0 == marker { skip = 1; next } skip { skip = 0; next } { print }' "$sudo_file" > "$new"
        as_root install "${owner[@]}" -m 0644 -- "$new" "$sudo_file"
        echo "Disabled; the previous file is $backup."
        ;;
    *) usage; exit 2 ;;
esac
