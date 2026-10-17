#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
DOJO_CUA_REV=88aff7e5424e9ed45b93ded85eb4111472ca5190
node vendor/cua/copy-upstream.mjs "${XDG_CACHE_HOME:-$HOME/.cache}/dojo/upstream/cua/$DOJO_CUA_REV" --verify
manifest=vendor/cua/upstream/libs/cua-driver/rust/Cargo.toml
cargo build --release --offline --locked --manifest-path "$manifest" -p cua-driver
DOJO_CUA_TARGET_DIR=$(cargo metadata --offline --no-deps --format-version 1 --manifest-path "$manifest" |
    node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(0, "utf8")).target_directory)')
export DOJO_CUA_TARGET_DIR
export DOJO_CUA_INSTALL_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin"
node --input-type=module <<'JS'
import { chmodSync, copyFileSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
const digest = file => createHash('sha256').update(readFileSync(file)).digest('hex');
const installDir = process.env.DOJO_CUA_INSTALL_DIR;
const binary = join(installDir, 'cua-driver');
mkdirSync(installDir, { recursive: true });
copyFileSync(join(process.env.DOJO_CUA_TARGET_DIR, 'release/cua-driver'), binary + '.new');
chmodSync(binary + '.new', 0o755);
renameSync(binary + '.new', binary);
const sources = ['vendor/cua/copy-plan.json', 'vendor/cua/copy-inventory.json', 'vendor/cua/metadata/niri-driver.patch', 'vendor/niri/niri-ipc/Cargo.toml', 'vendor/niri/niri-ipc/src/lib.rs', 'vendor/niri/niri-ipc/src/socket.rs', 'vendor/niri/niri-ipc/src/state.rs', 'crates/dojo-native-seat/Cargo.toml', 'vendor/cua/upstream/libs/cua-driver/rust/Cargo.lock', 'crates/dojo-native-seat/src/lib.rs'];
writeFileSync(binary + '.dojo.json', JSON.stringify({ upstream_commit: '88aff7e5424e9ed45b93ded85eb4111472ca5190', binary_sha256: digest(binary), sources: Object.fromEntries(sources.map(file => [file, digest(file)])) }, null, 2) + '\n');
console.log(binary);
JS