#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
fixture=$(mktemp -d /tmp/dojo-theme-policy-check.XXXXXX)
trap 'rm -rf -- "$fixture"' EXIT
managed=$fixture/policies/managed
mkdir -p "$managed"
chmod 0755 "$fixture" "$fixture/policies" "$managed"
policy='{"BrowserThemeColor": "#121916", "BrowserColorScheme": "system"}'
printf '%s\n' "$policy" >"$managed/dojo-theme.json"
printf '%s\n' "$policy" >"$managed/dojo-theme.json.backup-20261012105859-1234307"
printf '%s\n' '{"HomepageLocation":"https://example.invalid"}' >"$managed/unrelated.backup"
chmod 0644 "$managed"/*

run_helper() {
    bwrap --unshare-user --uid 0 --gid 0 --ro-bind / / \
        --bind "$fixture" /etc/chromium \
        /bin/bash "$repo_root/system-config/bin/dojo-chromium-theme-policy" "$1"
}

run_helper --clear
[[ $(find "$managed" -maxdepth 1 -type f | wc -l) -eq 1 ]]
[[ $(cat "$managed/unrelated.backup") == '{"HomepageLocation":"https://example.invalid"}' ]]
[[ $(find "$fixture/policies/dojo-backups" -maxdepth 1 -type f | wc -l) -eq 2 ]]
for backup in "$fixture/policies/dojo-backups"/*; do
    [[ $(cat "$backup") == "$policy" ]]
done
run_helper --clear
[[ $(find "$fixture/policies/dojo-backups" -maxdepth 1 -type f | wc -l) -eq 2 ]]
run_helper '#000b01'
[[ $(cat "$managed/dojo-theme.json") == '{"BrowserThemeColor": "#000b01", "BrowserColorScheme": "system"}' ]]

printf '%s\n' '{"BrowserThemeColor":"#ffffff"}' >"$managed/foreign.backup"
chmod 0644 "$managed/foreign.backup"
if run_helper '#112233' 2>/dev/null; then
    echo 'FAIL: non-json-suffix policy conflict was ignored' >&2
    exit 1
fi
[[ $(cat "$managed/dojo-theme.json") == '{"BrowserThemeColor": "#000b01", "BrowserColorScheme": "system"}' ]]
run_helper --clear
[[ -f $managed/foreign.backup && -f $managed/unrelated.backup ]]
[[ ! -e $managed/dojo-theme.json ]]
echo 'PASS: isolated helper clear/migration, unrelated policies, idempotence, exact colors, all-file conflicts'
