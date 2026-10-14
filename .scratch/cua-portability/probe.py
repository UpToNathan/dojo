#!/usr/bin/env python3
"""Extract selected CUA code into standalone crates for a portability check."""
import argparse
import shutil
import hashlib
import subprocess
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument('source', type=Path)
parser.add_argument('output', type=Path)
args = parser.parse_args()
revision = subprocess.check_output(['git', '-C', str(args.source), 'rev-parse', 'HEAD'], text=True).strip()
assert revision == '88aff7e5424e9ed45b93ded85eb4111472ca5190', revision
crates = args.source / 'libs/cua-driver/rust/crates'
args.output.mkdir(parents=True, exist_ok=True)
motion = args.output / 'motion'
shutil.copytree(crates / 'cua-cursor-motion', motion, dirs_exist_ok=True)
(motion / 'Cargo.toml').write_text('''[package]
name = "cua-cursor-motion"
version = "0.0.0"
edition = "2021"
license = "MIT"
[features]
default = []
serde = ["dep:serde"]
[dependencies]
serde = { version = "1", features = ["derive"], optional = true }
[dev-dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
[[example]]
name = "export_golden"
required-features = ["serde"]
''')
helpers = args.output / 'helpers'
(helpers / 'src').mkdir(parents=True, exist_ok=True)
for module in ['image_utils', 'window_inspection', 'element_frame', 'capture_registry']:
    shutil.copyfile(crates / f'cua-driver-core/src/{module}.rs', helpers / f'src/{module}.rs')
(helpers / 'src/browser').mkdir(exist_ok=True)
for module in ['cdp_ws', 'mock_cdp']:
    shutil.copyfile(crates / f'cua-driver-core/src/browser/{module}.rs', helpers / f'src/browser/{module}.rs')
(helpers / 'src/lib.rs').write_text('pub mod image_utils;\npub mod window_inspection;\npub mod element_frame;\npub mod capture_registry;\npub mod browser { pub mod cdp_ws; #[cfg(test)] mod mock_cdp; }\n')
(helpers / 'Cargo.toml').write_text('''[package]
name = "cua-portable-helpers"
license = "MIT"
version = "0.0.0"
edition = "2021"
[dependencies]
anyhow = "1"
image = { version = "0.25", default-features = false, features = ["png", "jpeg"] }
serde_json = "1"
sha2 = "0.10"
uuid = { version = "1", features = ["v4"] }
thiserror = "1"
tokio = { version = "1", features = ["full"] }
tokio-tungstenite = "0.24"
futures-util = "0.3"
''')
print(motion / 'Cargo.toml')
print(helpers / 'Cargo.toml')

for destination in [motion, helpers]:
    shutil.copyfile(args.source / 'LICENSE.md', destination / 'LICENSE.md')
for original in (crates / 'cua-cursor-motion/src').rglob('*.rs'):
    copied = motion / original.relative_to(crates / 'cua-cursor-motion')
    assert hashlib.sha256(original.read_bytes()).digest() == hashlib.sha256(copied.read_bytes()).digest()
for copied in (helpers / 'src').rglob('*.rs'):
    if copied.name == 'lib.rs':
        continue
    original = crates / 'cua-driver-core/src' / copied.relative_to(helpers / 'src')
    assert original.read_bytes() == copied.read_bytes()
print('Copied implementation files match upstream bytes; MIT notice retained.')
