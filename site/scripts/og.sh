#!/usr/bin/env bash
# Renders site/scripts/og.html (the site's own CSS, fonts and Capsule cards) with headless Chromium
# at 1200×630 and writes site/assets/og.png, the social share image.
#
# Needs: bun (to serve site/ locally), a headless Chromium (CHROME, else chromium/google-chrome on
# PATH, else Playwright's chrome-headless-shell), optionally oxipng/pngquant to shrink the PNG.
# Usage: site/scripts/og.sh
set -euo pipefail

site="$(cd "$(dirname "$0")/.." && pwd)"
port="${OG_PORT:-8799}"

chrome="${CHROME:-$(command -v chromium || command -v google-chrome-stable || command -v google-chrome || ls -d "$HOME"/.cache/ms-playwright/chromium_headless_shell-*/*/chrome-headless-shell 2>/dev/null | sort -V | tail -1 || true)}"
[ -x "$chrome" ] || { echo "no headless Chromium found; set CHROME" >&2; exit 1; }

bun --eval "
const root = '$site'
Bun.serve({ port: $port, fetch(r) {
  const p = new URL(r.url).pathname
  return new Response(Bun.file(root + (p === '/' ? '/scripts/og.html' : p)))
} })" &
server=$!
trap 'kill $server 2>/dev/null' EXIT
for _ in $(seq 50); do curl -sf "http://127.0.0.1:$port/" >/dev/null && break; sleep 0.1; done

tmp="$(mktemp -d)"
"$chrome" --headless --disable-gpu --hide-scrollbars --force-device-scale-factor=1 \
  --window-size=1200,630 --virtual-time-budget=3000 --screenshot="$tmp/og.png" \
  "http://127.0.0.1:$port/" 2>/dev/null

if command -v pngquant >/dev/null; then pngquant --quality 85-98 --speed 1 --force --output "$tmp/og.png" "$tmp/og.png"; fi
if command -v oxipng >/dev/null; then oxipng -q -o 4 --strip safe "$tmp/og.png"; fi
mv "$tmp/og.png" "$site/assets/og.png"
rm -rf "$tmp"
ls -la "$site/assets/og.png"
