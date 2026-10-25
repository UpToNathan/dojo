#!/usr/bin/env bash
# Installs 1Password's command line at /usr/local/bin/op with the setgid group it needs to talk to the
# desktop app (a Nix-store binary cannot carry that bit; see docs/research/1password-linux-autostart-resume.md).
# The mesh's onboarding makes its keys through `op`. Safe to repeat; a replaced file is kept as a backup.
set -euo pipefail

target=${DOJO_OP_TARGET:-/usr/local/bin/op}
group=onepassword-cli

usage() { echo "Usage: $0 [--from PATH]   (default: the op in the Nix profile)" >&2; }

source_file=
while [[ $# -gt 0 ]]; do
    case $1 in
        --from) [[ $# -ge 2 ]] || { usage; exit 2; }; source_file=$2; shift 2 ;;
        -h|--help) usage; exit 0 ;;
        *) usage; exit 2 ;;
    esac
done
if [[ -z $source_file ]]; then
    source_file=$(readlink -f "$HOME/.nix-profile/bin/op" 2>/dev/null || true)
fi
[[ -n $source_file && -x $source_file ]] || { echo 'No op to install: give one with --from PATH.' >&2; exit 1; }

as_root() {
    if [[ $EUID -eq 0 || -n ${DOJO_NO_SUDO:-} ]]; then "$@"; else sudo "$@"; fi
}
owner=(--owner=root "--group=$group")
[[ -z ${DOJO_NO_SUDO:-} ]] || owner=()

[[ -n ${DOJO_NO_SUDO:-} ]] || as_root groupadd --force "$group"
if [[ -f $target ]] && cmp -s -- "$source_file" "$target" && [[ $(stat -c %a "$target") == 2755 ]]; then
    echo "$target is already installed."
else
    if [[ -e $target ]]; then
        backup="$target.backup-$(date +%Y%m%d%H%M%S)"
        as_root mv -- "$target" "$backup"
        echo "Backed up $target to $backup"
    fi
    as_root install "${owner[@]}" --mode=2755 -- "$source_file" "$target"
    echo "Installed $target"
fi
"$target" --version
