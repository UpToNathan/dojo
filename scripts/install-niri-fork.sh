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
pin_executable() {
    local variable=$1 name=$2 executable owner mode direct_agent=false
    executable=$(readlink -e -- "${!variable:-}" || true)
    if [[ $name == dojo-agent && $(dirname -- "$executable") == /nix/store && $(basename -- "$executable") == *-dojo-agent ]]; then
        direct_agent=true
    fi
    if [[ ( $executable != /nix/store/*/bin/$name && $direct_agent != true ) || ! -f $executable || ! -x $executable ]]; then
        printf '%s must pin an installed Nix-store %s.\n' "$variable" "$name" >&2
        return 1
    fi
    owner=$(stat -Lc '%u' -- "$executable")
    mode=$(stat -Lc '%a' -- "$executable")
    if [[ $owner != 0 ]] || (( (8#$mode & 8#222) != 0 )); then
        printf 'The pinned %s must be root-owned and immutable.\n' "$name" >&2
        return 1
    fi
    printf '%s' "$executable"
}
broker=$(pin_executable DOJO_BROWSER_CONTROL_BROKER_EXE dojo-browser-control-broker)
mcp=""
if [[ -n ${DOJO_BROWSER_CONTROL_MCP_EXE:-} ]]; then
    mcp=$(pin_executable DOJO_BROWSER_CONTROL_MCP_EXE browser-control-mcp)
fi
semantic_agent=""
if [[ -n ${DOJO_SEMANTIC_AGENT_EXE:-} ]]; then
    semantic_agent=$(pin_executable DOJO_SEMANTIC_AGENT_EXE dojo-agent)
    if [[ $(od -An -tx1 -N4 -- "$semantic_agent" | tr -d ' \n') != 7f454c46 ]]; then
        printf 'DOJO_SEMANTIC_AGENT_EXE must pin a native ELF executable.\n' >&2
        exit 1
    fi
fi
perception=""
if [[ -n ${DOJO_PERCEPTION_INSTALLATION:-} ]]; then
    perception=$(readlink -e -- "$DOJO_PERCEPTION_INSTALLATION" || true)
    if [[ $perception != /nix/store/*-dojo-perception-installation.json || ! -f $perception ]]; then
        printf 'DOJO_PERCEPTION_INSTALLATION must pin a staged Nix-store dojo-perception-installation.json.\n' >&2
        exit 1
    fi
    if [[ $(stat -Lc '%u' -- "$perception") != 0 ]] || (( (8#$(stat -Lc '%a' -- "$perception") & 8#222) != 0 )); then
        printf 'The pinned perception installation must be root-owned and immutable.\n' >&2
        exit 1
    fi
fi
if ldd "$binary" | grep -q 'not found'; then
    printf 'The compositor has unresolved runtime libraries.\n' >&2
    exit 1
fi
"$binary" validate --config "$config_home/niri/config.kdl"

revision=$(
    { sha256sum "$binary"; printf '%s\n' "$LD_LIBRARY_PATH" "$broker" "$mcp" "$semantic_agent" "$perception"; } |
        sha256sum | cut -c1-16
)
installation_root="$data_home/dojo/niri"
installation="$installation_root/$revision"
mkdir -p "$installation/roots"
if [[ ! -f "$installation/niri" ]] || ! cmp -s "$binary" "$installation/niri"; then
    install -m 0755 "$binary" "$installation/niri.new-$$"
    mv -f -- "$installation/niri.new-$$" "$installation/niri"
fi
printf 'export LD_LIBRARY_PATH=%q\n' "$LD_LIBRARY_PATH" > "$installation/environment.sh"
printf 'export DOJO_BROWSER_CONTROL_BROKER_EXE=%q\n' "$broker" >> "$installation/environment.sh"
if [[ -n $mcp ]]; then
    printf 'export DOJO_BROWSER_CONTROL_MCP_EXE=%q\n' "$mcp" >> "$installation/environment.sh"
fi
if [[ -n $semantic_agent ]]; then
    printf 'export DOJO_SEMANTIC_AGENT_EXE=%q\n' "$semantic_agent" >> "$installation/environment.sh"
fi
if [[ -n $perception ]]; then
    printf 'export DOJO_PERCEPTION_INSTALLATION=%q\n' "$perception" >> "$installation/environment.sh"
fi
chmod 0644 "$installation/environment.sh"

# Keep both linked libraries and dynamically loaded graphics libraries alive.
mapfile -t store_paths < <(
    { ldd "$binary"; printf '%s\n' "$LD_LIBRARY_PATH" "$broker" "$mcp" "$semantic_agent" "$perception"; } |
        grep -oE '/nix/store/[^/[:space:]:]+' | sort -u
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
    # Home Manager owns some of these paths once it has run. Replacing its link would make the
    # next deployment refuse to overwrite the file.
    if [[ -L $target_file && $(readlink -f "$target_file") == /nix/store/* ]]; then
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
link_with_backup "$repo_root/scripts/dojo-agent" "$HOME/.local/bin/dojo-agent"
link_with_backup "$repo_root/session-config/90-dojo-niri.conf" "$config_home/systemd/user/wayland-wm@niri\\x2dsession.service.d/90-dojo-niri.conf"
systemctl --user daemon-reload
"$repo_root/scripts/prune-installations.sh" "$installation_root" 4 --apply
printf 'Installed %s\nThe next Niri UWSM login will use the fork. The current session is still running.\n' "$installation"
