#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 || ! $1 =~ ^(snapshot|check|restore)$ ]]; then
    printf 'usage: %s {snapshot|check|restore} WIDTHS.json\n' "$0" >&2
    exit 2
fi

action=$1
snapshot_file=$2
command -v jq >/dev/null || {
    printf 'jq is required; run this helper inside the Dojo devenv shell.\n' >&2
    exit 1
}

windows=$(niri msg --json windows)

case "$action" in
    snapshot)
        mkdir -p "$(dirname "$snapshot_file")"
        jq '[.[]
            | select(.is_floating == false)
            | select(.workspace_id != null)
            | select(.layout.pos_in_scrolling_layout != null)
            | {
                workspace_id,
                column_index: .layout.pos_in_scrolling_layout[0],
                window_id: .id,
                width: (.layout.tile_size[0] | round)
              }
          ] | unique_by([.workspace_id, .column_index])' \
            <<<"$windows" >"$snapshot_file"
        printf 'Saved tiled column widths to %s\n' "$snapshot_file"
        ;;
    check)
        failures=0
        while IFS=$'\t' read -r workspace_id column_index width; do
            expected=$((width - 76))
            if ((expected < 120)); then
                expected=120
            fi
            actual=$(jq -r \
                --argjson workspace "$workspace_id" \
                --argjson column "$column_index" \
                '[.[]
                 | select(.is_floating == false and .workspace_id == $workspace)
                 | select(.layout.pos_in_scrolling_layout[0] == $column)
                 | (.layout.tile_size[0] | round)]
                 | first // "missing"' <<<"$windows")
            printf 'workspace %s column %s: %s -> %s px (expected %s)\n' \
                "$workspace_id" "$column_index" "$width" "$actual" "$expected"
            if [[ $actual != "$expected" ]]; then
                failures=$((failures + 1))
            fi
        done < <(jq -r '.[] | [.workspace_id, .column_index, .width] | @tsv' "$snapshot_file")

        if ((failures > 0)); then
            printf '%s column width check(s) failed.\n' "$failures" >&2
            exit 1
        fi
        printf 'Verified tiled column widths.\n'
        ;;
    restore)
        while IFS=$'\t' read -r workspace_id column_index width; do
            window_id=$(jq -r \
                --argjson workspace "$workspace_id" \
                --argjson column "$column_index" \
                '[.[]
                 | select(.is_floating == false and .workspace_id == $workspace)
                 | select(.layout.pos_in_scrolling_layout[0] == $column)
                 | .id]
                 | first // "missing"' <<<"$windows")
            if [[ $window_id == missing ]]; then
                printf 'Skipped closed/moved workspace %s column %s.\n' \
                    "$workspace_id" "$column_index" >&2
                continue
            fi
            niri msg action set-window-width --id "$window_id" "$width"
            printf 'Restored workspace %s column %s to %s px.\n' \
                "$workspace_id" "$column_index" "$width"
        done < <(jq -r '.[] | [.workspace_id, .column_index, .width] | @tsv' "$snapshot_file")
        ;;
esac
