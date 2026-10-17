#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if (( $# > 1 )); then
    printf 'usage: %s [BINARY]\n' "$0" >&2
    exit 2
fi
binary=${1:-"$repo_root/target/release/dojo"}
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
if [[ $data_home != /* || $config_home != /* ]]; then
    printf 'XDG data and configuration paths must be absolute.\n' >&2
    exit 1
fi
if [[ ! -x "$binary" || -z "${LD_LIBRARY_PATH:-}" ]]; then
    printf 'Build the release desktop shell and run this installer inside devenv.\n' >&2
    exit 1
fi
if ldd "$binary" | grep -q 'not found'; then
    printf 'The desktop shell has unresolved runtime libraries.\n' >&2
    exit 1
fi
"$binary" --help >/dev/null

# The lock screen ships in the same installation: dojo-lock finds dojo-lock-pam
# beside itself. Never run dojo-lock here, it takes the session lock.
lock_binary="$(dirname -- "$binary")/dojo-lock"
lock_helper="$(dirname -- "$binary")/dojo-lock-pam"
for lock_file in "$lock_binary" "$lock_helper"; do
    if [[ ! -x $lock_file ]]; then
        printf 'Missing %s. Build the lock screen with: cargo build --release --bin dojo --bin dojo-lock --bin dojo-lock-pam\n' "$lock_file" >&2
        exit 1
    fi
done
if ldd "$lock_binary" | grep -q 'not found'; then
    printf 'The lock screen has unresolved runtime libraries.\n' >&2
    exit 1
fi
"$repo_root/scripts/check-dojo-lock-helper.py" "$lock_helper" || {
    printf 'The lock screen PAM helper does not work on this host; nothing was installed.\n' >&2
    exit 1
}

revision="$(sha256sum "$binary" | cut -c1-16)-mmap128k"
installation_root="$data_home/dojo/desktop"
installation="$installation_root/$revision"
mkdir -p "$installation/roots"
if [[ ! -f "$installation/dojo" ]] || ! cmp -s "$binary" "$installation/dojo"; then
    install -m 0755 "$binary" "$installation/dojo.new-$$"
    mv -f -- "$installation/dojo.new-$$" "$installation/dojo"
fi
for lock_file in "$lock_binary" "$lock_helper"; do
    installed_lock="$installation/$(basename -- "$lock_file")"
    if [[ ! -f $installed_lock ]] || ! cmp -s "$lock_file" "$installed_lock"; then
        install -m 0755 "$lock_file" "$installed_lock.new-$$"
        mv -f -- "$installed_lock.new-$$" "$installed_lock"
    fi
done
printf 'export LD_LIBRARY_PATH=%q\n' "$LD_LIBRARY_PATH" > "$installation/environment.sh"
cat >> "$installation/environment.sh" <<'ENV'
case ":${GLIBC_TUNABLES:-}:" in
    *:glibc.malloc.mmap_threshold=*:*) ;;
    *) export GLIBC_TUNABLES="${GLIBC_TUNABLES:+$GLIBC_TUNABLES:}glibc.malloc.mmap_threshold=131072" ;;
esac
ENV
chmod 0644 "$installation/environment.sh"

# Keep both linked libraries and dynamically loaded graphics libraries alive.
mapfile -t store_paths < <(
    { ldd "$binary"; printf '%s\n' "$LD_LIBRARY_PATH"; } |
        grep -o -E '/nix/store/[^/[:space:]:]+' | sort -u
)
for store_path in "${store_paths[@]}"; do
    nix-store --add-root "$installation/roots/$(basename "$store_path")" --indirect --realise "$store_path" >/dev/null
done
mapfile -t lock_store_paths < <(ldd "$lock_binary" "$lock_helper" | grep -o -E '/nix/store/[^/[:space:]:]+' | sort -u)
for store_path in "${lock_store_paths[@]}"; do
    nix-store --add-root "$installation/roots/$(basename "$store_path")" --indirect --realise "$store_path" >/dev/null
done

link_with_backup() {
    local source_file=$1 target_file=$2
    mkdir -p "$(dirname "$target_file")"
    if [[ -L $target_file && $(readlink -f "$target_file") == "$source_file" ]]; then
        return
    fi
    if [[ -e $target_file || -L $target_file ]]; then
        mv -- "$target_file" "$target_file.backup-$(date +%Y%m%d%H%M%S)-$$"
    fi
    ln -s -- "$source_file" "$target_file"
}

if [[ -L "$installation_root/current" ]]; then
    previous=$(readlink -f "$installation_root/current")
    if [[ $previous != "$installation" ]]; then
        ln -sfn -- "$previous" "$installation_root/previous"
    fi
elif [[ -e "$installation_root/current" ]]; then
    printf 'Refusing to replace a non-symlink desktop shell installation.\n' >&2
    exit 1
fi
ln -s -- "$installation" "$installation_root/current.new-$$"
mv -Tf -- "$installation_root/current.new-$$" "$installation_root/current"
link_with_backup "$repo_root/scripts/dojo-desktop" "$HOME/.local/bin/dojo-desktop"
link_with_backup "$repo_root/scripts/dojo-lock" "$HOME/.local/bin/dojo-lock"
link_with_backup "$repo_root/session-config/90-dojo-release.conf" "$config_home/systemd/user/app-dojo@autostart.service.d/90-dojo-release.conf"
systemctl --user daemon-reload
"$repo_root/scripts/prune-installations.sh" "$installation_root" 4 --apply
printf 'Installed %s\nRestart the Dojo service to activate it; the compositor stays running.\n' "$installation"
