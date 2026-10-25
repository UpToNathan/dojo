#!/usr/bin/env bash
# Onboards this machine into a Dojo mesh that uses the signed roster (docs/mesh.md): sshd for the
# mesh (public keys only, on its own port), then `dojo-mesh onboard` for the keys, the signing keys,
# trust and enrolment. bootstrap-host runs it as its last step on a new machine. Every step is safe
# to repeat. It asks for what it needs; give the answers as options to skip the questions.
#
#   scripts/onboard-mesh.sh                       # asks
#   scripts/onboard-mesh.sh --sign yubikey --sign 1password --route 100.64.0.7
#
# --sign names the keys this machine signs the roster with, in the order to try them: the first is
# the primary. Without --sign this machine only asks to be enrolled and writes a request for a machine
# that signs.
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
systemctl=${DOJO_SYSTEMCTL:-systemctl}
ssh_dir=${DOJO_SSH_DIR:-/etc/ssh}

usage() {
    cat >&2 <<'USAGE'
Usage: onboard-mesh.sh [--name NAME] [--route ADDRESS]... [--port PORT] [--sign yubikey|1password|KEY]...
                       [--vault NAME] [--no-host] [--yes]
  --name NAME      this machine's name in the mesh (default: its host name)
  --route ADDRESS  an address the other machines reach this one at (default: ask; the tailnet address if Tailscale is up)
  --port PORT      the port for the mesh's sshd (default 2222; not 22, which Tailscale SSH answers on a tailnet address)
  --sign KEY       a key this machine signs the roster with; repeat for more, primary first
  --vault NAME     the 1Password vault for the new keys
  --no-host        leave sshd alone (it is set up already, or another way in is used)
  --yes            do not ask to confirm
USAGE
}

name=""
port=2222
routes=()
signs=()
vault=""
host_setup=true
assume_yes=false
while [[ $# -gt 0 ]]; do
    case $1 in
        --name) [[ $# -ge 2 ]] || { usage; exit 2; }; name=$2; shift 2 ;;
        --route) [[ $# -ge 2 ]] || { usage; exit 2; }; routes+=("$2"); shift 2 ;;
        --port)
            [[ $# -ge 2 && $2 =~ ^[0-9]+$ && $2 -ge 1 && $2 -le 65535 ]] || { echo '--port needs a port number from 1 to 65535' >&2; exit 2; }
            port=$2; shift 2 ;;
        --sign) [[ $# -ge 2 ]] || { usage; exit 2; }; signs+=("$2"); shift 2 ;;
        --vault) [[ $# -ge 2 ]] || { usage; exit 2; }; vault=$2; shift 2 ;;
        --no-host) host_setup=false; shift ;;
        --yes) assume_yes=true; shift ;;
        -h|--help) usage; exit 0 ;;
        *) usage; exit 2 ;;
    esac
done

interactive=false
[[ -t 0 && -t 1 ]] && interactive=true

ask() { # ask PROMPT DEFAULT -> the answer
    local answer
    read -r -p "$1 [$2] " answer || answer=
    printf '%s' "${answer:-$2}"
}

dojo_mesh=${DOJO_MESH:-${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh}
[[ -x $dojo_mesh ]] || dojo_mesh=$(command -v dojo-mesh || true)
[[ -x $dojo_mesh ]] || { echo 'dojo-mesh is not installed: run scripts/install-helpers.sh first.' >&2; exit 1; }

command -v op >/dev/null 2>&1 || { echo "1Password's command line (op) is not installed: run scripts/install-onepassword-cli.sh, then this again." >&2; exit 1; }

# Keys are made in 1Password and used through its SSH agent, so it has to be ready before anything is changed.
agent=${DOJO_AGENT_SOCKET:-$HOME/.1password/agent.sock}
if ! SSH_AUTH_SOCK=$agent ssh-add -L >/dev/null 2>&1; then
    echo "1Password's SSH agent does not answer ($agent)." >&2
    echo 'Unlock 1Password on this machine and turn on Settings > Developer > Use the SSH agent, then run this again.' >&2
    exit 3
fi

if [[ -z $name ]]; then
    name=$(tr '[:upper:]' '[:lower:]' < /proc/sys/kernel/hostname)
    ! $interactive || name=$(ask 'Name of this machine in the mesh?' "$name")
fi

if [[ ${#signs[@]} -eq 0 ]] && $interactive; then
    echo 'Which keys does this machine sign the roster with? Name them in the order to try them (the first is'
    echo 'the primary): yubikey, 1password, or both. Leave "none" if another machine signs for this one.'
    read -r -a signs < <(printf '%s\n' "$(ask 'Signing keys?' 'yubikey 1password')" | tr ',' ' ')
    [[ ${signs[0]:-none} != none ]] || signs=()
fi

if [[ ${#routes[@]} -eq 0 ]]; then
    guess=$(tailscale ip -4 2>/dev/null | head -n1 || true)
    [[ -n $guess ]] || guess=$(ip -4 -o addr show scope global 2>/dev/null | awk '{sub("/.*", "", $4); print $4; exit}')
    if $interactive; then
        routes=("$(ask 'Address the other machines reach this one at?' "${guess:-}")")
    else
        routes=("$guess")
    fi
fi
[[ -n ${routes[0]:-} ]] || { echo 'No address for this machine: give one with --route.' >&2; exit 2; }
# Routes carry the port the mesh's sshd listens on, unless the person wrote one.
for index in "${!routes[@]}"; do
    [[ ${routes[$index]} == *:* ]] || routes[$index]="${routes[$index]}:$port"
done

echo "This machine will join the mesh as '$name', reached at ${routes[*]}."
if [[ ${#signs[@]} -gt 0 ]]; then echo "It signs the roster with: ${signs[*]} (first = primary)."; else echo 'It signs nothing; another machine signs it in.'; fi
if $interactive && ! $assume_yes; then
    [[ $(ask 'Go ahead?' 'Y') =~ ^[Yy] ]] || { echo 'Nothing changed.'; exit 0; }
fi

if $host_setup; then
    echo '== sshd'
    # No authorized key exists yet on a new machine: sshd then admits nobody until the roster lets members in.
    "$repo_root/scripts/install-mesh-sshd.sh" --port "${routes[0]##*:}" --allow-no-keys
    if ! "$systemctl" is-active --quiet sshd.service 2>/dev/null; then
        if [[ $EUID -eq 0 || -n ${DOJO_NO_SUDO:-} ]]; then "$systemctl" enable --now sshd.service; else sudo "$systemctl" enable --now sshd.service; fi
    fi
    host_key=${DOJO_HOST_KEY:-$ssh_dir/ssh_host_ed25519_key.pub}
    for _ in $(seq 1 20); do [[ -r $host_key ]] && break; sleep 0.5; done
    [[ -r $host_key ]] || { echo "sshd made no host key at $host_key." >&2; exit 1; }
fi

echo '== mesh'
args=(onboard --node-name "$name")
for route in "${routes[@]}"; do args+=(--route "$route"); done
for sign in "${signs[@]}"; do args+=(--sign "$sign"); done
[[ -z $vault ]] || args+=(--vault "$vault")
[[ -z ${DOJO_HOST_KEY:-} ]] || args+=(--host-key-file "$DOJO_HOST_KEY")
"$dojo_mesh" "${args[@]}"

case ${routes[0]%:*} in
    100.6[4-9].*|100.[7-9][0-9].*|100.1[01][0-9].*|100.12[0-7].*) ;;
    *) echo "Note: ${routes[0]%:*} is not a tailnet address. If this machine's firewall drops that network, open port ${routes[0]##*:}/tcp on it (firewall-cmd --permanent --zone=ZONE --add-port=${routes[0]##*:}/tcp)." ;;
esac
