#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
DOJO_CUA_REV=88aff7e5424e9ed45b93ded85eb4111472ca5190
node vendor/cua/copy-upstream.mjs "${XDG_CACHE_HOME:-$HOME/.cache}/dojo/upstream/cua/$DOJO_CUA_REV" --verify
export CARGO_TARGET_DIR="${XDG_CACHE_HOME:-$HOME/.cache}/dojo/cua-niri-target"
cargo build --offline --locked --manifest-path vendor/cua/upstream/libs/cua-driver/rust/Cargo.toml -p cua-driver
node --input-type=module <<'JS'
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
const digest = file => createHash('sha256').update(readFileSync(file)).digest('hex');
const binary = join(process.env.CARGO_TARGET_DIR, 'debug/cua-driver');
const sources = ['vendor/cua/copy-plan.json', 'vendor/cua/copy-inventory.json', 'vendor/cua/metadata/niri-driver.patch', 'vendor/niri/niri-ipc/Cargo.toml', 'vendor/niri/niri-ipc/src/lib.rs', 'vendor/niri/niri-ipc/src/socket.rs', 'vendor/niri/niri-ipc/src/state.rs', 'crates/dojo-native-seat/Cargo.toml', 'vendor/cua/upstream/libs/cua-driver/rust/Cargo.lock', 'crates/dojo-native-seat/src/lib.rs'];
writeFileSync(binary + '.dojo.json', JSON.stringify({ upstream_commit: '88aff7e5424e9ed45b93ded85eb4111472ca5190', binary_sha256: digest(binary), sources: Object.fromEntries(sources.map(file => [file, digest(file)])) }, null, 2) + '\n');
console.log(binary);
JS
