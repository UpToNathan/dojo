//! CUA Linux browser identity/listener evidence helpers.
//! MIT, Cua AI, Inc.; source provenance is recorded in platform-extraction.json.
use std::os::unix::fs::MetadataExt;
use super::types::{BrowserProduct, BrowserProcessRole};

pub(super) fn is_chromium_identity(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let products = [
        "chrome", "chromium", "electron", "brave", "edge", "msedge", "vivaldi", "opera", "arc",
        "thorium", "iridium", "yandex", "helium",
    ];
    name.split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| products.contains(&token))
}

pub(super) fn browser_product(identity: &str) -> BrowserProduct {
    let tokens = identity
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let has = |values: &[&str]| tokens.iter().any(|token| values.contains(&token.as_str()));
    if has(&["google"]) && has(&["chrome"]) {
        BrowserProduct::GoogleChrome
    } else if has(&["msedge"]) || (has(&["microsoft"]) && has(&["edge"])) {
        BrowserProduct::MicrosoftEdge
    } else if has(&["chromium", "helium"]) {
        BrowserProduct::Chromium
    } else if has(&["brave"]) {
        BrowserProduct::Brave
    } else if has(&["vivaldi"]) {
        BrowserProduct::Vivaldi
    } else if has(&["opera"]) {
        BrowserProduct::Opera
    } else if has(&["electron"]) {
        BrowserProduct::Electron
    } else if has(&["firefox"]) {
        BrowserProduct::Firefox
    } else {
        BrowserProduct::Other
    }
}

pub(super) fn trusted_root_owned_installation(executable: &std::path::Path) -> bool {
    if !executable.is_absolute() {
        return false;
    }
    let mut current = Some(executable);
    while let Some(path) = current {
        let Ok(metadata) = std::fs::symlink_metadata(path) else {
            return false;
        };
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || (metadata.mode() & 0o022 != 0 && !(metadata.is_dir() && metadata.mode() & 0o1000 != 0))
        {
            return false;
        }
        current = path.parent();
    }
    std::fs::metadata(executable)
        .is_ok_and(|metadata| metadata.is_file() && metadata.mode() & 0o111 != 0)
}

pub(super) fn process_role_for_pid(pid: i64, product: BrowserProduct) -> BrowserProcessRole {
    let helper = std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .is_some_and(|bytes| {
            bytes.split(|byte| *byte == 0).any(|arg| {
                arg.starts_with(b"--type=") && arg.get(7..).is_some_and(|kind| !kind.is_empty())
            })
        });
    if helper {
        BrowserProcessRole::Helper
    } else if product == BrowserProduct::Electron {
        BrowserProcessRole::EmbeddedApplication
    } else if matches!(
        product,
        BrowserProduct::GoogleChrome
            | BrowserProduct::Chromium
            | BrowserProduct::MicrosoftEdge
            | BrowserProduct::Brave
            | BrowserProduct::Vivaldi
            | BrowserProduct::Opera
            | BrowserProduct::Arc
    ) {
        BrowserProcessRole::StandaloneConsumer
    } else {
        BrowserProcessRole::Unknown
    }
}

pub(super) fn parse_proc_net_loopback_listeners(text: &str) -> Vec<(u16, u64)> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let fields = line.split_whitespace().collect::<Vec<_>>();
            if fields.len() < 10 || fields[3] != "0A" {
                return None;
            }
            let (address, port) = fields[1].split_once(':')?;
            let loopback = address == "0100007F" || address == "00000000000000000000000001000000";
            if !loopback {
                return None;
            }
            Some((
                u16::from_str_radix(port, 16).ok()?,
                fields[9].parse::<u64>().ok()?,
            ))
        })
        .collect()
}
