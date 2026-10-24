//! What the launcher has launched, and the searches that found it.
//!
//! Each launch adds weight to an application that halves every ten days (frecency), and the text
//! typed to find it is remembered for a while, so typing `ff` can reach Firefox first. The model
//! is pure: callers pass the time, and the store reads and writes one JSON file under the state
//! directory.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

const DAY_MS: f64 = 86_400_000.0;
const HALF_LIFE_DAYS: f64 = 10.0;
/// Weight one launch adds, in frecency points; a launch just now scores 101.
const LAUNCH_WEIGHT: f64 = 100.0;
/// Learned search terms stop counting this long after the last launch (17 days).
pub const TERM_WINDOW_MS: u64 = 17 * 86_400_000;
const MAX_TERMS: usize = 20;
const MAX_ENTRIES: usize = 500;
/// `exp` overflows past this; the score is clamped well before it matters.
const MAX_EXPONENT: f64 = 700.0;

/// How a typed query relates to the searches that previously led to one application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TermMatch {
    None,
    /// The query is the start of a learned term (`f` for a learned `ff`).
    Prefix,
    /// The query is a learned term.
    Exact,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Term {
    term: String,
    at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    /// Moment (ms since the epoch) at which the score decays to 1. It sits in the future after a
    /// launch, which keeps the score a single number that needs no per-launch log.
    anchor_ms: f64,
    opened_at_ms: u64,
    terms: Vec<Term>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LaunchHistory {
    entries: BTreeMap<String, Entry>,
}

fn decay_per_ms() -> f64 {
    std::f64::consts::LN_2 / (HALF_LIFE_DAYS * DAY_MS)
}

fn score_at(anchor_ms: f64, now_ms: f64) -> f64 {
    (decay_per_ms() * (anchor_ms - now_ms))
        .min(MAX_EXPONENT)
        .exp()
        .max(1.0)
}

fn normalize(text: &str) -> String {
    text.trim().to_lowercase()
}

impl LaunchHistory {
    /// Frecency of an application: 1 when it has never been launched or has fully decayed.
    pub fn frecency(&self, id: &str, now_ms: u64) -> f64 {
        self.entries
            .get(id)
            .map_or(1.0, |entry| score_at(entry.anchor_ms, now_ms as f64))
    }

    /// Whether the query matches a search that previously led to this application. Terms only
    /// count while the application still has weight and was launched within the window.
    pub fn term_match(&self, id: &str, query: &str, now_ms: u64) -> TermMatch {
        let Some(entry) = self.entries.get(id) else {
            return TermMatch::None;
        };
        let query = normalize(query);
        if query.is_empty()
            || score_at(entry.anchor_ms, now_ms as f64) <= 1.0 + f64::EPSILON
            || now_ms.saturating_sub(entry.opened_at_ms) > TERM_WINDOW_MS
        {
            return TermMatch::None;
        }
        entry
            .terms
            .iter()
            .map(|learned| {
                if learned.term == query {
                    TermMatch::Exact
                } else if learned.term.len() > query.len() && learned.term.starts_with(&query) {
                    TermMatch::Prefix
                } else {
                    TermMatch::None
                }
            })
            .max()
            .unwrap_or(TermMatch::None)
    }

    /// Notes that `id` was launched at `now_ms` after `query` was typed.
    pub fn record(&mut self, id: &str, query: &str, now_ms: u64) {
        let now = now_ms as f64;
        let entry = self.entries.entry(id.to_owned()).or_insert(Entry {
            anchor_ms: now,
            opened_at_ms: now_ms,
            terms: Vec::new(),
        });
        let next = score_at(entry.anchor_ms, now) + LAUNCH_WEIGHT;
        entry.anchor_ms = now + next.ln() / decay_per_ms();
        entry.opened_at_ms = now_ms;
        let query = normalize(query);
        if !query.is_empty() {
            entry.terms.retain(|learned| learned.term != query);
            entry.terms.push(Term {
                term: query,
                at_ms: now_ms,
            });
            let excess = entry.terms.len().saturating_sub(MAX_TERMS);
            entry.terms.drain(..excess);
        }
        self.prune(now_ms);
    }

    /// Drops what no longer affects ranking, then the lightest entries past the cap.
    fn prune(&mut self, now_ms: u64) {
        self.entries.retain(|_, entry| {
            entry.anchor_ms > now_ms as f64
                || now_ms.saturating_sub(entry.opened_at_ms) <= TERM_WINDOW_MS
        });
        if self.entries.len() > MAX_ENTRIES {
            let mut ranked: Vec<_> = self
                .entries
                .iter()
                .map(|(id, entry)| (id.clone(), entry.anchor_ms))
                .collect();
            ranked.sort_by(|left, right| right.1.total_cmp(&left.1));
            for (id, _) in ranked.into_iter().skip(MAX_ENTRIES) {
                self.entries.remove(&id);
            }
        }
    }

    /// A missing file is an empty history. A damaged one is too, so a bad write never keeps the
    /// launcher from opening.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|error| {
                eprintln!(
                    "Dojo ignored the launch history at {}: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                eprintln!(
                    "Dojo could not read the launch history at {}: {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("launch history path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let contents = serde_json::to_vec(self).context("serializing the launch history")?;
        let mut file = fs::File::create(&temporary)
            .with_context(|| format!("creating {}", temporary.display()))?;
        file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("writing {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("replacing {} with {}", path.display(), temporary.display()))
    }
}

/// `$XDG_STATE_HOME/dojo/launch-history.json`, beside the rest of Dojo's state.
pub fn default_path() -> Result<PathBuf> {
    let state_home = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) => PathBuf::from(path),
        None => std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("neither XDG_STATE_HOME nor HOME is set")?
            .join(".local/state"),
    };
    if !state_home.is_absolute() {
        bail!("XDG_STATE_HOME must be absolute");
    }
    Ok(state_home.join("dojo/launch-history.json"))
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

/// Reads the default history, records one launch and writes it back.
pub fn record_launch(id: &str, query: &str) -> Result<()> {
    let path = default_path()?;
    let mut history = LaunchHistory::load(&path);
    history.record(id, query, now_ms());
    history.save(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400_000;
    const NOW: u64 = 1_800_000_000_000;

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < expected * 0.001,
            "{actual} is not close to {expected}"
        );
    }

    #[test]
    fn unknown_applications_have_the_floor_score() {
        assert_eq!(LaunchHistory::default().frecency("a.desktop", NOW), 1.0);
    }

    #[test]
    fn a_launch_scores_101_and_halves_every_ten_days() {
        let mut history = LaunchHistory::default();
        history.record("a.desktop", "", NOW);
        assert_close(history.frecency("a.desktop", NOW), 101.0);
        assert_close(history.frecency("a.desktop", NOW + 10 * DAY), 50.5);
        assert_close(history.frecency("a.desktop", NOW + 20 * DAY), 25.25);
        assert_eq!(history.frecency("a.desktop", NOW + 400 * DAY), 1.0);
    }

    #[test]
    fn launches_accumulate_and_recent_use_beats_old_use() {
        let mut history = LaunchHistory::default();
        history.record("often.desktop", "", NOW);
        history.record("often.desktop", "", NOW);
        history.record("old.desktop", "", NOW - 30 * DAY);
        history.record("new.desktop", "", NOW - DAY);
        assert_close(history.frecency("often.desktop", NOW), 201.0);
        assert!(history.frecency("new.desktop", NOW) > history.frecency("old.desktop", NOW));
    }

    #[test]
    fn learned_terms_match_exactly_or_by_prefix_and_ignore_case() {
        let mut history = LaunchHistory::default();
        history.record("firefox.desktop", "  FF ", NOW);
        history.record("firefox.desktop", "browser", NOW);
        assert_eq!(
            history.term_match("firefox.desktop", "ff", NOW),
            TermMatch::Exact
        );
        assert_eq!(
            history.term_match("firefox.desktop", "bro", NOW),
            TermMatch::Prefix
        );
        assert_eq!(
            history.term_match("firefox.desktop", "f", NOW),
            TermMatch::Prefix
        );
        assert_eq!(
            history.term_match("firefox.desktop", "fx", NOW),
            TermMatch::None
        );
        assert_eq!(
            history.term_match("firefox.desktop", "", NOW),
            TermMatch::None
        );
        assert_eq!(
            history.term_match("other.desktop", "ff", NOW),
            TermMatch::None
        );
    }

    #[test]
    fn learned_terms_expire_after_the_window() {
        let mut history = LaunchHistory::default();
        history.record("firefox.desktop", "ff", NOW);
        assert_eq!(
            history.term_match("firefox.desktop", "ff", NOW + 17 * DAY),
            TermMatch::Exact
        );
        assert_eq!(
            history.term_match("firefox.desktop", "ff", NOW + 17 * DAY + 1),
            TermMatch::None
        );
    }

    #[test]
    fn terms_are_deduplicated_and_capped_at_the_newest_twenty() {
        let mut history = LaunchHistory::default();
        for number in 0..30 {
            history.record("a.desktop", &format!("term{number}"), NOW + number);
        }
        history.record("a.desktop", "term29", NOW + 31);
        let terms = &history.entries["a.desktop"].terms;
        assert_eq!(terms.len(), MAX_TERMS);
        assert_eq!(terms.first().map(|t| t.term.as_str()), Some("term10"));
        assert_eq!(terms.last().map(|t| t.term.as_str()), Some("term29"));
    }

    #[test]
    fn fully_decayed_entries_are_dropped_on_the_next_launch() {
        let mut history = LaunchHistory::default();
        history.record("stale.desktop", "x", NOW);
        history.record("fresh.desktop", "y", NOW + 400 * DAY);
        assert!(!history.entries.contains_key("stale.desktop"));
        assert!(history.entries.contains_key("fresh.desktop"));
    }

    #[test]
    fn history_round_trips_and_damage_or_absence_means_empty() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("nested/launch-history.json");
        assert_eq!(LaunchHistory::load(&path), LaunchHistory::default());

        let mut history = LaunchHistory::default();
        history.record("a.desktop", "ff", NOW);
        history.save(&path)?;
        assert_eq!(LaunchHistory::load(&path), history);

        fs::write(&path, "{ not json")?;
        assert_eq!(LaunchHistory::load(&path), LaunchHistory::default());
        Ok(())
    }
}
