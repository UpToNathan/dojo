#!/usr/bin/env bash
set -euo pipefail
spike_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$spike_dir"
sha256sum --check --quiet imported.sha256
sha256sum --check --quiet patched.sha256
cargo check --manifest-path Cargo.toml -p cua-driver-core --no-default-features
cargo test --manifest-path Cargo.toml -p cua-driver-core --no-default-features --lib embedded::
cargo test --manifest-path Cargo.toml -p cua-driver-core --no-default-features --lib browser::
cargo test --manifest-path Cargo.toml -p cua-driver-core --no-default-features --lib snapshot_store::
cargo test --manifest-path Cargo.toml -p cua-driver-core --no-default-features --doc embedded
