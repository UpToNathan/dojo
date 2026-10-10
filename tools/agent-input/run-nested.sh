#!/usr/bin/env bash
set -euo pipefail
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
niri_binary=${NIRI_BINARY:-"$repo_root/vendor/niri/target/debug/niri"}
run_directory=$(mktemp -d "${TMPDIR:-/tmp}/dojo-agent-nested.XXXXXX")
niri_pid=
cleanup() {
    if [[ -n "$niri_pid" ]]; then
        kill "$niri_pid" 2>/dev/null || true
        wait "$niri_pid" 2>/dev/null || true
    fi
    printf 'Nested-session evidence: %s\n' "$run_directory"
}
trap cleanup EXIT INT TERM
if [[ ! -x "$niri_binary" ]]; then
    printf 'Missing compositor binary: %s\n' "$niri_binary" >&2
    exit 1
fi
DOJO_AGENT_INPUT=1 RUST_LOG=niri=info "$niri_binary" --config "$repo_root/tools/agent-input/nested.kdl" >"$run_directory/niri.log" 2>&1 &
niri_pid=$!
for _ in {1..200}; do
    if ! kill -0 "$niri_pid" 2>/dev/null; then
        cat "$run_directory/niri.log" >&2
        exit 1
    fi
    child_display=$(sed -n 's/.*listening on Wayland socket: \([^[:space:]]*\).*/\1/p' "$run_directory/niri.log" | tail -n 1)
    child_socket=$(sed -n 's/.*IPC listening on: \([^[:space:]]*\).*/\1/p' "$run_directory/niri.log" | tail -n 1)
    if [[ -n "$child_display" && -n "$child_socket" && -S "$child_socket" ]]; then
        break
    fi
    sleep 0.05
done
if [[ -z "$child_display" || -z "$child_socket" || ! -S "$child_socket" ]]; then
    cat "$run_directory/niri.log" >&2
    exit 1
fi
printf 'WAYLAND_DISPLAY=%q\nNIRI_SOCKET=%q\n' "$child_display" "$child_socket" >"$run_directory/session.env"
driver="$repo_root/tools/agent-input/run.py"
if [[ "${1:-}" == --mcp ]]; then
    shift
    driver="$repo_root/tools/agent-input/run-mcp.py"
fi
WAYLAND_DISPLAY="$child_display" NIRI_SOCKET="$child_socket" python3 "$driver" --log "$run_directory/events.jsonl" "$@"
