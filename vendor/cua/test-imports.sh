#!/usr/bin/env bash
# Run from Dojo root through devenv. No live desktop, installs or downloads.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/dojo-cua-adoption-target}"
node vendor/cua/verify-import.mjs
node --test vendor/cua/verify-import.test.mjs
cargo test --offline --locked --manifest-path crates/dojo-cua-primitives/Cargo.toml
cargo test --offline --locked --manifest-path vendor/cua/crates/cua-cursor-motion/Cargo.toml --all-features
cargo test --offline --locked --manifest-path vendor/cua/crates/cua-spaces-transcript/Cargo.toml
