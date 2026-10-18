#!/usr/bin/env bash
# Record this host's proprietary NVIDIA driver for the Home Manager deployment.
#
# Programs started from the Nix store carry Nix's glibc and cannot load Arch's
# NVIDIA libraries. Home Manager (targets.genericLinux.gpu.nvidia) rebuilds the
# user-space half of the driver from NVIDIA's own installer, and it has to match
# the loaded kernel module exactly. home-manager.nix reads the version and hash from
# /etc/dojo/nvidia-driver.json, which this script writes. Hosts without the NVIDIA
# module have no such file and are unaffected.
#
# Run it again after the NVIDIA driver package updates, then redeploy.
set -euo pipefail

export PATH=/nix/var/nix/profiles/default/bin:$HOME/.nix-profile/bin:$PATH

as_root() {
    if [[ $EUID -eq 0 ]]; then
        "$@"
    else
        sudo "$@"
    fi
}

version=$(modinfo -F version nvidia 2>/dev/null || true)
if [[ -z $version ]]; then
    printf 'No nvidia kernel module found; this host does not need the NVIDIA driver record.\n' >&2
    exit 1
fi

url="https://download.nvidia.com/XFree86/Linux-x86_64/$version/NVIDIA-Linux-x86_64-$version.run"
printf 'NVIDIA driver %s: hashing %s\n' "$version" "$url"
hash=$(nix store prefetch-file --json "$url" | python3 -I -c 'import json, sys; print(json.load(sys.stdin)["hash"])')

record=$(printf '{\n  "version": "%s",\n  "sha256": "%s"\n}\n' "$version" "$hash")
as_root install -d -m 0755 /etc/dojo
printf '%s' "$record" | as_root tee /etc/dojo/nvidia-driver.json >/dev/null
printf 'Wrote /etc/dojo/nvidia-driver.json:\n%s\n' "$record"
printf 'Redeploy Home Manager, then run its non-nixos-gpu-setup as root to activate the driver tree.\n'
