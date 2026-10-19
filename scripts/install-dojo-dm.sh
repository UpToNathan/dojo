#!/usr/bin/env bash
# Install the Dojo display manager release under /usr/local/lib/dojo-dm.
#
# usage: install-dojo-dm.sh [BINARY_DIR]
#
# BINARY_DIR (default target/release) holds the built dojo-dm, dojo-dm-pam and dojo-greeter. The Dojo
# Niri fork and the desktop shell must already be installed (install-niri-fork.sh,
# install-dojo-release.sh): their installed environment.sh files supply the library path.
#
# Everything root runs or loads is a root-owned copy in /usr/local/lib/dojo-dm/<release>/, never a
# path in a home directory or a checkout. Nothing here starts through devenv, and the installed
# environment.sh names only /nix/store (GC-rooted) paths. Run it from devenv like the other
# installers; sudo is used only for the writes under /usr/local/lib.
#
# DOJO_DM_ROOT prefixes every installed path and turns sudo off, for tests. DOJO_DM_NIRI_INSTALLATION
# and DOJO_DM_DESKTOP_INSTALLATION override the installations whose environment is read.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
if (( $# > 1 )); then
    printf 'usage: %s [BINARY_DIR]\n' "$0" >&2
    exit 2
fi
binary_dir=${1:-"$repo_root/target/release"}
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
if [[ $data_home != /* ]]; then
    printf 'XDG_DATA_HOME must be absolute.\n' >&2
    exit 1
fi
niri_installation=${DOJO_DM_NIRI_INSTALLATION:-"$data_home/dojo/niri/current"}
desktop_installation=${DOJO_DM_DESKTOP_INSTALLATION:-"$data_home/dojo/desktop/current"}
prefix=${DOJO_DM_ROOT:-}
if [[ -n $prefix && $prefix != /* ]]; then
    printf 'DOJO_DM_ROOT must be absolute.\n' >&2
    exit 1
fi

if [[ -n $prefix || $EUID -eq 0 ]]; then
    as_root() { "$@"; }
else
    as_root() { sudo "$@"; }
fi
owner=()
[[ -n $prefix ]] || owner=(-o root -g root)

fail() {
    printf '%s\n' "$*" >&2
    exit 1
}

for name in dojo-dm dojo-dm-pam dojo-greeter; do
    [[ -f $binary_dir/$name && -x $binary_dir/$name ]] ||
        fail "Missing $binary_dir/$name. Build it with: cargo build --release --bin dojo-dm --bin dojo-dm-pam --bin dojo-greeter"
done
niri_dir=$(readlink -e -- "$niri_installation" || true)
desktop_dir=$(readlink -e -- "$desktop_installation" || true)
[[ -n $niri_dir && -f $niri_dir/niri && -x $niri_dir/niri && -f $niri_dir/environment.sh ]] ||
    fail 'The Dojo Niri fork is not installed. Run scripts/install-niri-fork.sh from devenv first.'
[[ -n $desktop_dir && -f $desktop_dir/environment.sh ]] ||
    fail 'The Dojo desktop shell is not installed. Run scripts/install-dojo-release.sh from devenv first.'
nix_store=$(command -v nix-store || true)
[[ -n $nix_store ]] || fail 'nix-store is not on PATH; the release needs GC roots for its libraries.'

stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT

# A clean shell reads only what the installed file itself exports.
installed_variable() {
    env -i bash -c '. "$1" >/dev/null && printf %s "${!2:-}"' _ "$1" "$2"
}

# Root loads these libraries, so an entry has to be an immutable store path or the root-generated
# graphics tree; a user-writable entry would be code execution as root.
library_path=""
declare -A seen=()
for installation in "$niri_dir" "$desktop_dir"; do
    value=$(installed_variable "$installation/environment.sh" LD_LIBRARY_PATH)
    IFS=: read -r -a entries <<< "$value"
    for entry in "${entries[@]}"; do
        [[ -n $entry && -z ${seen[$entry]:-} ]] || continue
        if [[ $entry != /nix/store/* && $entry != /run/opengl-driver/* ]] || [[ $entry == *..* ]]; then
            fail "Refusing library path entry $entry from $installation/environment.sh: only /nix/store paths are allowed."
        fi
        seen[$entry]=1
        library_path+=${library_path:+:}$entry
    done
done
[[ -n $library_path ]] || fail 'The installed Niri and desktop environments export no LD_LIBRARY_PATH; reinstall them from devenv.'
glibc_tunables=$(installed_variable "$desktop_dir/environment.sh" GLIBC_TUNABLES)
if [[ -n $glibc_tunables && ! $glibc_tunables =~ ^[A-Za-z0-9_.=:-]+$ ]]; then
    fail "Refusing GLIBC_TUNABLES value from $desktop_dir/environment.sh."
fi

# One read of each source, so the hash describes exactly what gets installed.
for name in dojo-dm dojo-dm-pam dojo-greeter; do
    cp -- "$binary_dir/$name" "$stage/$name"
done
cp -- "$niri_dir/niri" "$stage/niri"
cp -- "$repo_root/system-config/dojo-dm/greeter.kdl" "$stage/greeter.kdl.in"
chmod 0755 "$stage"/dojo-dm "$stage"/dojo-dm-pam "$stage"/dojo-greeter "$stage"/niri

cat > "$stage/run" <<'RUN'
#!/bin/sh
here=$(dirname "$(readlink -f "$0")")
. "$here/environment.sh"
exec "$here/dojo-dm" "$@"
RUN
chmod 0755 "$stage/run"
{
    printf 'export LD_LIBRARY_PATH=%q\n' "$library_path"
    [[ -z $glibc_tunables ]] || printf 'export GLIBC_TUNABLES=%q\n' "$glibc_tunables"
} > "$stage/environment.sh"
chmod 0644 "$stage/environment.sh"

for name in dojo-dm dojo-dm-pam dojo-greeter niri; do
    if LD_LIBRARY_PATH=$library_path ldd "$stage/$name" | grep -q 'not found'; then
        fail "$name has unresolved runtime libraries; nothing was installed."
    fi
done
"$repo_root/scripts/check-dojo-lock-helper.py" "$stage/dojo-dm-pam" ||
    fail 'The display manager PAM helper does not work on this host; nothing was installed.'

revision=$(
    cd "$stage" && sha256sum dojo-dm dojo-dm-pam dojo-greeter niri greeter.kdl.in run environment.sh | sha256sum | cut -c1-16
)
lib_dir=$prefix/usr/local/lib/dojo-dm
release_dir=$lib_dir/$revision
sed "s#@RELEASE@#/usr/local/lib/dojo-dm/$revision#g" "$stage/greeter.kdl.in" > "$stage/greeter.kdl"
chmod 0644 "$stage/greeter.kdl"
rm -- "$stage/greeter.kdl.in"
LD_LIBRARY_PATH=$library_path "$stage/niri" validate --config "$stage/greeter.kdl" ||
    fail 'The greeter Niri configuration is not valid; nothing was installed.'

release_files=(dojo-dm dojo-dm-pam dojo-greeter niri run environment.sh greeter.kdl)
release_is_current() {
    local name
    [[ -d $release_dir && ! -L $release_dir ]] || return 1
    if [[ -z $prefix && $(stat -c '%u' -- "$release_dir") != 0 ]]; then return 1; fi
    for name in "${release_files[@]}"; do
        [[ -f $release_dir/$name && ! -L $release_dir/$name ]] && cmp -s -- "$stage/$name" "$release_dir/$name" || return 1
        if [[ -z $prefix && $(stat -c '%u' -- "$release_dir/$name") != 0 ]]; then return 1; fi
    done
}

timestamp=$(date +%Y%m%d%H%M%S)-$$
if [[ -L $lib_dir || ( -e $lib_dir && ! -d $lib_dir ) ]]; then
    fail "Refusing unsafe installation directory $lib_dir."
fi
as_root install -d -m 0755 "${owner[@]}" -- "$lib_dir"

if release_is_current; then
    printf 'Release %s is already installed.\n' "$revision"
else
    if [[ -e $release_dir || -L $release_dir ]]; then
        as_root mv -- "$release_dir" "$release_dir.backup-$timestamp"
        printf 'Backed up existing release to %s\n' "$release_dir.backup-$timestamp"
    fi
    incoming=$lib_dir/$revision.new-$$
    as_root rm -rf -- "$incoming"
    as_root install -d -m 0755 "${owner[@]}" -- "$incoming"
    for name in "${release_files[@]}"; do
        mode=0644
        [[ -x $stage/$name ]] && mode=0755
        as_root install -m "$mode" "${owner[@]}" -- "$stage/$name" "$incoming/$name"
    done
    as_root mv -T -- "$incoming" "$release_dir"
fi

# The roots keep the linked libraries and the dynamically loaded graphics libraries alive. They
# are made after the release has its final name, because an indirect root records the link's path.
as_root install -d -m 0755 "${owner[@]}" -- "$release_dir/roots"
mapfile -t store_paths < <(
    {
        for name in dojo-dm dojo-dm-pam dojo-greeter niri; do
            LD_LIBRARY_PATH=$library_path ldd "$stage/$name"
        done
        printf '%s\n' "$library_path"
    } | grep -o -E '/nix/store/[^/[:space:]:]+' | sort -u
)
for store_path in "${store_paths[@]}"; do
    as_root "$nix_store" --add-root "$release_dir/roots/$(basename "$store_path")" --indirect --realise "$store_path" >/dev/null
done

if [[ -L $lib_dir/current ]]; then
    previous=$(basename -- "$(readlink -f -- "$lib_dir/current")")
    if [[ $previous != "$revision" && -d $lib_dir/$previous ]]; then
        as_root ln -sfn -- "$previous" "$lib_dir/previous"
    fi
elif [[ -e $lib_dir/current ]]; then
    as_root mv -- "$lib_dir/current" "$lib_dir/current.backup-$timestamp"
    printf 'Backed up existing current to %s\n' "$lib_dir/current.backup-$timestamp"
fi
as_root ln -s -- "$revision" "$lib_dir/current.new-$$"
as_root mv -Tf -- "$lib_dir/current.new-$$" "$lib_dir/current"

as_root "$repo_root/scripts/prune-installations.sh" "$lib_dir" 4 --apply
printf 'Installed %s\nThe next boot uses it; to switch now, leave the graphical login first, then restart dojo-dm.service from tty2.\n' "$release_dir"
