#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    printf 'usage: %s EVIDENCE_DIR\n' "$0" >&2
    exit 2
fi

evidence_dir=$(realpath -m "$1")
project_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)
devenv_root="$project_root"
pid_file="$evidence_dir/dojo-session.pid"

if pgrep -x dojo >/dev/null; then
    printf 'Dojo is already running; refusing to start a second instance.\n' >&2
    exit 1
fi

mkdir -p "$evidence_dir"
if [[ -e $pid_file ]]; then
    printf 'Refusing to reuse an evidence directory with an existing PID file: %s\n' "$pid_file" >&2
    exit 1
fi

setsid bash -c '
    cd "$1"
    export PATH="$HOME/.local/bin:$HOME/.local/share/mbx/bin:$HOME/.cargo/bin:$PATH"
    exec devenv shell -- cargo run --manifest-path "$2/Cargo.toml" -- --enable
' _ "$devenv_root" "$project_root" >"$evidence_dir/dojo.log" 2>&1 </dev/null &
printf '%s\n' "$!" >"$pid_file"
printf 'Started Dojo in process group %s; log: %s/dojo.log\n' "$(<"$pid_file")" "$evidence_dir"
