#!/usr/bin/env bash
# Remove old versioned installations that the Dojo installers leave behind.
#
# usage: prune-installations.sh ROOT [KEEP] [--apply]
#
# ROOT holds one directory per installed revision plus `current` and `previous`
# symlinks (for example ~/.local/share/dojo/desktop). The newest KEEP revisions
# (default 4), the revisions `current` and `previous` point at, and any revision
# that a running process executes from are kept. Without --apply this only lists
# what would be removed. Removing a revision also drops the Nix GC roots it
# holds, so run `nix-collect-garbage` afterwards to reclaim the store paths.
set -euo pipefail

apply=0
positional=()
for argument in "$@"; do
    if [[ $argument == --apply ]]; then apply=1; else positional+=("$argument"); fi
done
if (( ${#positional[@]} < 1 || ${#positional[@]} > 2 )); then
    printf 'usage: %s ROOT [KEEP] [--apply]\n' "$0" >&2
    exit 2
fi
root=${positional[0]%/}
keep=${positional[1]:-4}
if [[ $root != /* || ! -d $root || ! -L $root/current ]]; then
    printf 'Refusing: %s is not an absolute installation root with a current link.\n' "$root" >&2
    exit 1
fi
if ! [[ $keep =~ ^[0-9]+$ ]]; then
    printf 'KEEP must be a number.\n' >&2
    exit 2
fi

declare -A protected=()
protect() {
    local resolved
    resolved=$(readlink -f -- "$1" 2>/dev/null) || return 0
    [[ $resolved == "$root"/* ]] || return 0
    resolved=${resolved#"$root"/}
    protected[${resolved%%/*}]=1
}
protect "$root/current"
[[ -L $root/previous ]] && protect "$root/previous"
for exe in /proc/[0-9]*/exe; do
    target=$(readlink -- "$exe" 2>/dev/null || true)
    target=${target% (deleted)}
    [[ -n $target ]] && protect "$target"
done

mapfile -t revisions < <(
    find "$root" -mindepth 1 -maxdepth 1 -type d -printf '%T@ %f\n' | sort -rn | cut -d' ' -f2-
)
kept=0 removed=0 freed=0
for revision in "${revisions[@]}"; do
    if [[ -n ${protected[$revision]:-} ]] || (( kept < keep )); then
        kept=$((kept + 1))
        continue
    fi
    size=$(du -sk -- "$root/$revision" | cut -f1)
    freed=$((freed + size))
    removed=$((removed + 1))
    if (( apply )); then
        rm -rf -- "${root:?}/$revision"
    else
        printf 'would remove %s\n' "$root/$revision"
    fi
done
verb=$([[ $apply == 1 ]] && echo Removed || echo 'Would remove')
printf '%s %d revision(s) (%d MiB) from %s; kept %d.\n' "$verb" "$removed" "$((freed / 1024))" "$root" "$kept"
