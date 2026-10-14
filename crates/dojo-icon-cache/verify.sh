#!/usr/bin/env bash
set -euo pipefail
icon_cache_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$icon_cache_dir"
sha256sum --check --quiet provenance.sha256
cargo check --manifest-path Cargo.toml
cargo test --manifest-path Cargo.toml
