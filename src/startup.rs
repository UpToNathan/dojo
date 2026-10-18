//! Timeline of the shell's start-up, in milliseconds since `begin`. Read it from the journal
//! (`journalctl --user -u app-dojo@autostart.service | grep dojo-startup`) to see where a start
//! spends its time.

use std::sync::OnceLock;
use std::time::Instant;

static BEGUN: OnceLock<Instant> = OnceLock::new();

pub fn begin() {
    let _ = BEGUN.set(Instant::now());
}

pub fn mark(event: &str) {
    if let Some(begun) = BEGUN.get() {
        eprintln!("dojo-startup: event={event} ms={}", begun.elapsed().as_millis());
    }
}
