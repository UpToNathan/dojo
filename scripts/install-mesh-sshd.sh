#!/usr/bin/env bash
# Hardens this machine's sshd for the Dojo mesh: public keys only, no root login, and (optionally)
# listening only on the addresses given. The files are root-owned copies of system-config/ssh, and a
# replaced file is kept as a timestamped backup. The change is checked with `sshd -t` before sshd is
# told to reload, and undone if the check fails. Existing sessions are not interrupted.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
ssh_dir=${DOJO_SSH_DIR:-/etc/ssh}
sshd=${DOJO_SSHD:-sshd}
systemctl=${DOJO_SYSTEMCTL:-systemctl}

usage() {
    echo "Usage: $0 [--user NAME] [--listen ADDRESS]... [--allow-no-keys]" >&2
    echo "  --user NAME       the account whose authorized_keys must hold a key (default: the invoking user)" >&2
    echo "  --listen ADDRESS  listen only on this address (repeatable); without it sshd listens as it already does" >&2
    echo "  --allow-no-keys   harden even though NAME has no authorized key yet (sshd then admits nobody)" >&2
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

target_user=${SUDO_USER:-$(id -un)}
allow_no_keys=false
listen=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --user) [[ $# -ge 2 ]] || { usage; exit 2; }; target_user=$2; shift 2 ;;
        --listen)
            [[ $# -ge 2 && $2 =~ ^[A-Za-z0-9._:-]+$ ]] || { echo "--listen needs an address or host name" >&2; exit 2; }
            listen+=("$2"); shift 2 ;;
        --allow-no-keys) allow_no_keys=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage; exit 2 ;;
    esac
done

home=${DOJO_TARGET_HOME:-$(getent passwd "$target_user" | cut -d: -f6)}
[[ -n $home ]] || { echo "No such user: $target_user" >&2; exit 2; }
keys=$home/.ssh/authorized_keys
# With passwords off, a machine whose user has no authorized key admits nobody over SSH.
if ! grep -qsE '^[[:space:]]*[^#[:space:]]' -- "$keys" && ! $allow_no_keys; then
    echo "$keys holds no public key, so after this change sshd would admit nobody." >&2
    echo "Add one first (dojo-mesh roster authorize, or ssh-copy-id), or pass --allow-no-keys if" >&2
    echo "another way in (such as Tailscale SSH) is how you will reach this machine." >&2
    exit 1
fi

stamp=$(date +%Y%m%d%H%M%S)-$$
rendered=$(mktemp -d)
trap 'rm -rf -- "$rendered"' EXIT

# What would put the old files back, newest first.
undo=()
place() {
    local source=$1 target=$2
    as_root install -d -m 0755 "${owner[@]}" -- "$(dirname "$target")"
    if [[ -f $target && ! -L $target ]] && cmp -s -- "$source" "$target"; then
        return
    fi
    if [[ -e $target || -L $target ]]; then
        local backup="$target.backup-$stamp"
        as_root mv -- "$target" "$backup"
        printf 'Backed up existing file to %s\n' "$backup"
        undo+=("mv -- '$backup' '$target'")
    else
        undo+=("rm -f -- '$target'")
    fi
    as_root install "${owner[@]}" -m 0644 -- "$source" "$target"
    printf 'Installed %s (copy)\n' "$target"
}

rollback() {
    local step
    for ((i=${#undo[@]}-1; i>=0; i--)); do
        step=${undo[$i]}
        as_root bash -c "$step" || true
    done
}

hardening=$ssh_dir/sshd_config.d/10-dojo-mesh.conf
listening=$ssh_dir/sshd_config.d/11-dojo-mesh-listen.conf
place "$repo_root/system-config/ssh/sshd_config.d/10-dojo-mesh.conf" "$hardening"
if [[ ${#listen[@]} -gt 0 ]]; then
    {
        echo '# Dojo mesh: listen only where the mesh is meant to be reached (scripts/install-mesh-sshd.sh --listen).'
        printf 'ListenAddress %s\n' "${listen[@]}"
    } > "$rendered/listen.conf"
    place "$rendered/listen.conf" "$listening"
elif [[ -e $listening ]]; then
    backup="$listening.backup-$stamp"
    as_root mv -- "$listening" "$backup"
    undo+=("mv -- '$backup' '$listening'")
    printf 'Removed %s (kept as %s): sshd listens as its own configuration says\n' "$listening" "$backup"
fi

# An include that is missing means the drop-ins do nothing; say so instead of reporting success.
if ! grep -qsE '^[[:space:]]*Include[[:space:]].*sshd_config\.d' -- "$ssh_dir/sshd_config"; then
    echo "Warning: $ssh_dir/sshd_config does not include sshd_config.d, so the drop-ins are not read." >&2
fi

if ! as_root "$sshd" -t -f "$ssh_dir/sshd_config"; then
    echo "sshd rejected the new configuration; putting the old files back." >&2
    rollback
    exit 1
fi

unit=
for candidate in sshd.service ssh.service; do
    if "$systemctl" is-active --quiet "$candidate" 2>/dev/null; then unit=$candidate; break; fi
done
if [[ -n $unit ]]; then
    as_root "$systemctl" reload "$unit"
    printf 'Reloaded %s; open sessions continue.\n' "$unit"
else
    echo "sshd is not running, so nothing was reloaded; the next start reads the new files."
fi
echo "Check from another machine: scripts/check-mesh-sshd.sh THIS-MACHINE"
