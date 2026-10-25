#!/usr/bin/env bash
# Asks a machine's sshd what it would accept, from here, without a password to give: passes only
# when public keys are the one method offered. Run it from a different machine than the one checked.
set -euo pipefail

[[ $# -ge 1 && $# -le 2 ]] || { echo "Usage: $0 HOST [PORT]" >&2; exit 2; }
host=$1
port=${2:-22}

# The probe offers nothing: with BatchMode there is no prompt, and the refusal names the methods
# the server was willing to accept, which is what is being asked.
set +e
reply=$(ssh -F /dev/null -o BatchMode=yes -o PubkeyAuthentication=no \
    -o PreferredAuthentications=password,keyboard-interactive \
    -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o LogLevel=ERROR \
    -o ConnectTimeout=5 -p "$port" "dojo-probe@$host" true 2>&1)
set -e

methods=$(sed -n 's/.*Permission denied (\([^)]*\)).*/\1/p' <<<"$reply" | head -n1)
if [[ -z $methods ]]; then
    printf 'Could not tell what %s accepts: %s\n' "$host" "${reply:-no answer}" >&2
    exit 2
fi
if [[ ,$methods, == *,password,* || ,$methods, == *,keyboard-interactive,* ]]; then
    printf '%s still accepts: %s\n' "$host" "$methods" >&2
    exit 1
fi
printf '%s accepts only: %s\n' "$host" "$methods"
