#!/usr/bin/env bash
# Renders Hokum's settings window with the real app and writes site/assets/settings.{webp,avif}
# (and a 1x WebP), the one picture on hokum.app.
#
# Rendering needs a GPU, so the build and the snapshot run on a remote host (default `apollo`,
# override with HOKUM_GPU_HOST). The ref (default origin/dev) is pushed to a throwaway clone
# there and built with its own target dir; the snapshot runs with a throwaway XDG_CONFIG_HOME
# and HOME (a fresh config: Capsule), so the host's installed Hokum and ~/.config/hokum are never touched.
# Everything under /tmp/hokum-settings-* on the host is removed afterwards.
#
# Needs locally: git, ssh, scp, ffmpeg (libwebp; libaom for AVIF is optional).
# Usage: site/scripts/settings-shot.sh [ref]
set -euo pipefail

host="${HOKUM_GPU_HOST:-apollo}"
ref="${1:-origin/dev}"
root="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
out="$root/site/assets"
src=/tmp/hokum-settings-src
target=/tmp/hokum-settings-target
shot=/tmp/hokum-settings-shot

ssh "$host" "rm -rf $src $shot && git init -q $src"
git -C "$root" push -q -f "$host:$src" "$ref:refs/heads/shot"
ssh "$host" "git -C $src checkout -q shot && cd $src && CARGO_TARGET_DIR=$target cargo build --release -q"
# A throwaway HOME too, so the Files section reads ~/.config/hokum/… rather than a /tmp path.
ssh "$host" "mkdir -p $shot/home/.config/hokum && HOME=$shot/home XDG_CONFIG_HOME=$shot/home/.config $target/release/hokum --snap $shot/settings.png --scene hokum --scale 2 --theme dark"

tmp="$(mktemp -d)"
scp -q "$host:$shot/settings.png" "$tmp/settings.png"
ssh "$host" "rm -rf $src $shot $target"

ffmpeg -loglevel error -y -i "$tmp/settings.png" -c:v libwebp -quality 86 -compression_level 6 "$out/settings.webp"
ffmpeg -loglevel error -y -i "$tmp/settings.png" -vf "scale=iw/2:-1:flags=lanczos" -c:v libwebp -quality 86 -compression_level 6 "$out/settings-1x.webp"
if ffmpeg -hide_banner -encoders 2>/dev/null | grep -q libaom-av1; then
  ffmpeg -loglevel error -y -i "$tmp/settings.png" -c:v libaom-av1 -still-picture 1 -crf 30 -cpu-used 4 -pix_fmt yuv420p "$out/settings.avif" || rm -f "$out/settings.avif"
fi
ffprobe -v error -show_entries stream=width,height -of csv=p=0 "$tmp/settings.png"
rm -rf "$tmp"
ls -la "$out"/settings*
