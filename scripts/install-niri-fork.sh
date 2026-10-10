#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
binary="$repo_root/vendor/niri/target/release/niri"
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
if [[ $data_home != /* || $config_home != /* ]]; then
    printf 'XDG data and configuration paths must be absolute.\n' >&2
    exit 1
fi
if [[ ! -x "$binary" || -z "${LD_LIBRARY_PATH:-}" ]]; then
    printf 'Build the release compositor and run this installer inside devenv.\n' >&2
    exit 1
fi
if ldd "$binary" | rg -q 'not found'; then
    printf 'The compositor has unresolved runtime libraries.\n' >&2
    exit 1
fi
"$binary" validate --config "$config_home/niri/config.kdl"

revision=$(sha256sum "$binary" | cut -c1-16)
installation_root="$data_home/dojo/niri"
installation="$installation_root/$revision"
mkdir -p "$installation/roots"
if [[ ! -f "$installation/niri" ]] || ! cmp -s "$binary" "$installation/niri"; then
    install -m 0755 "$binary" "$installation/niri.new-$$"
    mv -f -- "$installation/niri.new-$$" "$installation/niri"
fi
printf 'export LD_LIBRARY_PATH=%q\n' "$LD_LIBRARY_PATH" > "$installation/environment.sh"
chmod 0644 "$installation/environment.sh"

# Keep both linked libraries and dynamically loaded graphics libraries alive.
mapfile -t store_paths < <(
    { ldd "$binary"; printf '%s\n' "$LD_LIBRARY_PATH"; } |
        rg -o '/nix/store/[^/[:space:]:]+' | sort -u
)
for store_path in "${store_paths[@]}"; do
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
    printf 'Refusing to replace a non-symlink compositor installation.\n' >&2
    exit 1
fi
ln -s -- "$installation" "$installation_root/current.new-$$"
mv -Tf -- "$installation_root/current.new-$$" "$installation_root/current"
link_with_backup "$repo_root/scripts/dojo-niri-session" "$HOME/.local/bin/dojo-niri-session"
link_with_backup "$repo_root/session-config/90-dojo-niri.conf" "$config_home/systemd/user/wayland-wm@niri\\x2dsession.service.d/90-dojo-niri.conf"
systemctl --user daemon-reload
printf 'Installed %s\nThe next Niri UWSM login will use the fork. The current session is still running.\n' "$installation"
