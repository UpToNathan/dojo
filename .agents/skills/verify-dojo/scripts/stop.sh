#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    printf 'usage: %s EVIDENCE_DIR\n' "$0" >&2
    exit 2
fi

evidence_dir=$(realpath -m "$1")
pid_file="$evidence_dir/dojo-session.pid"
width_file="$evidence_dir/window-widths.json"

if [[ ! -f $pid_file ]]; then
    printf 'No Dojo session PID file at %s\n' "$pid_file" >&2
    exit 1
fi
if [[ ! -f $width_file ]]; then
    printf 'No saved Niri width snapshot at %s\n' "$width_file" >&2
    exit 1
fi

session_pid=$(<"$pid_file")
if [[ ! $session_pid =~ ^[0-9]+$ ]]; then
    printf 'Invalid session PID in %s\n' "$pid_file" >&2
    exit 1
fi

if kill -0 -- "-$session_pid" 2>/dev/null; then
    kill -INT -- "-$session_pid"
    for _ in {1..100}; do
        kill -0 -- "-$session_pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 -- "-$session_pid" 2>/dev/null; then
        kill -TERM -- "-$session_pid"
        for _ in {1..50}; do
            kill -0 -- "-$session_pid" 2>/dev/null || break
            sleep 0.1
        done
    fi
fi

if kill -0 -- "-$session_pid" 2>/dev/null; then
    printf 'Dojo process group %s did not stop; widths were not restored.\n' "$session_pid" >&2
    exit 1
fi

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
devenv_root=$(cd "$script_dir/../../../.." && pwd)
(cd "$devenv_root" && devenv shell -- "$script_dir/layout-widths.sh" restore "$width_file")
rm -f "$pid_file"
