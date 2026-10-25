//! A record of every remote call this node makes, one JSON line each, appended before the call
//! (so a crash still leaves a trace) and again with its result.

use crate::transport::Origin;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

const DETAIL_LIMIT: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub at: DateTime<Utc>,
    pub origin: Origin,
    pub from: String,
    pub session: String,
    pub peer: String,
    /// `exec`, `cp` or `task`.
    pub action: String,
    pub detail: String,
    /// `started`, then `exit 0`, `exit 1`, `needs-human`, `timed-out` or `refused: …`.
    pub result: String,
}

impl Entry {
    pub fn new(origin: Origin, from: &str, session: &str, peer: &str, action: &str, detail: &str, result: &str, at: DateTime<Utc>) -> Self {
        Self {
            at,
            origin,
            from: from.into(),
            session: session.into(),
            peer: peer.into(),
            action: action.into(),
            detail: truncate(detail),
            result: result.into(),
        }
    }
}

fn truncate(text: &str) -> String {
    if text.len() <= DETAIL_LIMIT {
        return text.to_string();
    }
    let mut end = DETAIL_LIMIT;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

pub fn path(state_dir: &Path) -> PathBuf {
    state_dir.join("audit.jsonl")
}

pub fn append(state_dir: &Path, entry: &Entry) -> std::io::Result<()> {
    std::fs::create_dir_all(state_dir)?;
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path(state_dir))?;
    let mut line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    line.push('\n');
    // One write call so concurrent calls do not interleave their lines.
    file.write_all(line.as_bytes())
}

/// The last `count` entries, oldest first. A line that cannot be read is skipped, not fatal.
pub fn last(state_dir: &Path, count: usize) -> std::io::Result<Vec<Entry>> {
    let text = match std::fs::read_to_string(path(state_dir)) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let entries: Vec<Entry> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    let skip = entries.len().saturating_sub(count);
    Ok(entries.into_iter().skip(skip).collect())
}

pub fn render(entry: &Entry) -> String {
    format!(
        "{} {} {}@{} -> {} {} {} [{}]",
        entry.at.format("%Y-%m-%d %H:%M:%S"),
        entry.origin.as_str(),
        entry.session,
        entry.from,
        entry.peer,
        entry.action,
        entry.detail,
        entry.result
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-10T12:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-mesh-audit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn appends_and_reads_back_the_last_entries_in_order() {
        let dir = dir("order");
        for n in 0..5 {
            append(&dir, &Entry::new(Origin::Agent, "alpha", "s", "beta", "exec", &format!("cmd {n}"), "started", at())).unwrap();
        }
        let entries = last(&dir, 2).unwrap();
        assert_eq!(entries.iter().map(|e| e.detail.as_str()).collect::<Vec<_>>(), ["cmd 3", "cmd 4"]);
        assert_eq!(last(&dir, 99).unwrap().len(), 5);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_log_is_empty_and_a_torn_line_is_skipped() {
        let dir = dir("torn");
        assert_eq!(last(&dir, 5).unwrap(), []);
        append(&dir, &Entry::new(Origin::Operator, "a", "s", "b", "cp", "x", "exit 0", at())).unwrap();
        std::fs::OpenOptions::new().append(true).open(path(&dir)).unwrap().write_all(b"{\"at\": broken\n").unwrap();
        assert_eq!(last(&dir, 5).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn long_details_are_cut_on_a_character_boundary() {
        let entry = Entry::new(Origin::Agent, "a", "s", "b", "exec", &"é".repeat(3000), "started", at());
        assert!(entry.detail.len() <= DETAIL_LIMIT + '…'.len_utf8());
        assert!(entry.detail.ends_with('…'));
    }

    #[test]
    fn renders_one_readable_line() {
        let entry = Entry::new(Origin::Agent, "alpha", "s1", "beta", "exec", "ls", "exit 0", at());
        assert_eq!(render(&entry), "2026-01-10 12:00:00 agent s1@alpha -> beta exec ls [exit 0]");
    }
}
