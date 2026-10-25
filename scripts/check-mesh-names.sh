#!/usr/bin/env bash
# Fails if the mesh's tracked source, tests or docs name this machine's real devices, tailnet or
# user. Names come from the machine running the check, so the check itself names no one. Without
# Tailscale there is nothing to look for and it passes.
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cd "$repo_root"

command -v jq >/dev/null 2>&1 || { printf 'check-mesh-names: jq is required; run inside devenv.\n' >&2; exit 1; }

paths=(crates/dojo-mesh docs/mesh.md docs/adr/0005-tailscale-identity-is-node-identity-and-permission.md .scratch/mesh)
names=()
if command -v tailscale >/dev/null 2>&1 && report=$(tailscale status --json 2>/dev/null); then
    mapfile -t names < <(
        jq -r '[.MagicDNSSuffix, .CurrentTailnet.Name, (.Self, (.Peer // {})[]) | .HostName?] | .[] | strings | select(length >= 3)' <<<"$report" | sort -u
    )
fi
names+=("${USER:-}")

failed=0
for name in "${names[@]}"; do
    [[ ${#name} -ge 3 ]] || continue
    # git grep lists only tracked files; --untracked adds new ones. Build output is untracked and ignored.
    if git grep -n --untracked --exclude-standard -i -w -F -e "$name" -- "${paths[@]}"; then
        printf 'check-mesh-names: %q appears above; mesh source must use invented names.\n' "$name" >&2
        failed=1
    fi
done
exit "$failed"
