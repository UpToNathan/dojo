#!/usr/bin/env bash
# Installs release builds of the voice service helpers outside the checkout so
# the service keeps working after `target/` is cleaned. Run inside devenv.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
if [[ $data_home != /* ]]; then
    printf 'XDG_DATA_HOME must be absolute.\n' >&2
    exit 1
fi
if (( $# > 0 )); then
    helpers=("$@")
else
    helpers=(dojo-speech dojo-computer-recording dojo-computer-verification dojo-voice-ctl)
fi
install_dir="$data_home/dojo/bin"
mkdir -p "$install_dir/roots"

for name in "${helpers[@]}"; do
    source_file="$repo_root/target/release/$name"
    if [[ $name == dojo-voice-ctl || $name == dojo-speech ]]; then
        # Standalone crates, each with its own lockfile and target directory.
        manifest="$repo_root/crates/$name/Cargo.toml"
        cargo build --release --locked --manifest-path "$manifest"
        source_file="$(cargo metadata --offline --locked --no-deps --format-version 1 --manifest-path "$manifest" |
            jq -r .target_directory)/release/$name"
    fi
    if [[ ! -x $source_file ]]; then
        printf 'Missing %s. Build it first with cargo build --release (see docs/agent-input.md).\n' "$source_file" >&2
        exit 1
    fi
    if ldd "$source_file" | grep -q 'not found'; then
        printf '%s has unresolved runtime libraries; run this inside devenv.\n' "$name" >&2
        exit 1
    fi
    install -m 0755 "$source_file" "$install_dir/$name.new-$$"
    mv -f -- "$install_dir/$name.new-$$" "$install_dir/$name"

    # Keep linked and dynamically loaded libraries from being garbage collected.
    mapfile -t store_paths < <(
        { ldd "$source_file"; printf '%s\n' "${LD_LIBRARY_PATH:-}"; } |
            grep -o -E '/nix/store/[^/[:space:]:]+' | sort -u
    )
    for store_path in "${store_paths[@]}"; do
        nix-store --add-root "$install_dir/roots/$(basename "$store_path")" --indirect --realise "$store_path" >/dev/null
    done
    printf 'Installed %s\n' "$install_dir/$name"
done

# The voice service starts without the devenv shell, which used to supply the audio plugin paths
# dojo-speech needs to record. Keep the plugin directory alive and record the paths.
if [[ -n ${ALSA_CONFIG_PATH:-} && -n ${ALSA_PLUGIN_DIR:-} ]]; then
    {
        printf 'export ALSA_CONFIG_PATH=%q\n' "$ALSA_CONFIG_PATH"
        printf 'export ALSA_PLUGIN_DIR=%q\n' "$ALSA_PLUGIN_DIR"
    } > "$install_dir/speech-environment.sh.new-$$"
    mv -f -- "$install_dir/speech-environment.sh.new-$$" "$install_dir/speech-environment.sh"
    plugin_store_path=$(printf '%s\n' "$ALSA_PLUGIN_DIR" | grep -o -E '/nix/store/[^/[:space:]:]+' || true)
    if [[ -n $plugin_store_path ]]; then
        nix-store --add-root "$install_dir/roots/$(basename "$plugin_store_path")" --indirect --realise "$plugin_store_path" >/dev/null
    fi
    printf 'Installed %s\n' "$install_dir/speech-environment.sh"
fi
