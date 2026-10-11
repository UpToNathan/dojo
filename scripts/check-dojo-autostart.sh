#!/usr/bin/env bash
set -euo pipefail

unit=app-dojo@autostart.service
systemctl --user is-active --quiet "$unit" || {
    systemctl --user show "$unit" -p ActiveState -p ExecMainStatus
    exit 1
}

pid=$(systemctl --user show "$unit" --property=MainPID --value)
[[ $pid != 0 && $(cat "/proc/$pid/comm") == dojo ]] || {
    printf 'The autostart service has no running Dojo process.\n' >&2
    exit 1
}
[[ $(pgrep -cx dojo) == 1 ]] || {
    printf 'Expected exactly one Dojo process.\n' >&2
    exit 1
}

session_socket=$(systemctl --user show-environment | sed -n 's/^NIRI_SOCKET=//p')
layers=$(NIRI_SOCKET="$session_socket" niri msg --json layers)
jq -e 'any(.[]; .namespace | startswith("dojo-frame-"))' <<<"$layers" >/dev/null || {
    printf 'Dojo is running but its desktop frame is not mapped.\n' >&2
    exit 1
}
printf 'Dojo autostart is active with PID %s and a mapped desktop frame.\n' "$pid"
