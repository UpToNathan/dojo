//! 04 Clipboard History (reads what clipvault records). See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! # What clipvault gives us
//!
//! Dojo's `clipvault-history` user service runs `wl-paste --watch clipvault store`, which keeps a
//! SQLite database at `~/.local/share/clipvault.db` (one `clipboard` table: id, content blob
//! UNIQUE, last_updated, content_type, mimetype, extra_preview_data). The schema is clipvault's
//! own, so this module never opens the file. It uses the stable surface instead, the CLI:
//!
//! * `clipvault list -m 300 -f id,preview,size,last-updated` prints one tab-separated line per
//!   entry, newest first. Previews have whitespace collapsed and end in `…` when cut to the
//!   width; binary entries read `[[ binary data 26kB image/png ]]` and entries that are not UTF-8
//!   read `[[ Non-UTF-8 text ]]`. The size is rounded (`4kB`) and the time is whole-second UTC.
//! * `clipvault get <id>` prints the entry's bytes; `clipvault delete <id>` removes one entry.
//! * Storing the same content again keeps its id and moves its time to now, so a re-copy through
//!   `wl-copy` (which `wl-paste --watch` records) moves the entry to the top by itself.
//! * Ids are SQLite rowids: when the newest entry is deleted its id can be handed to the next
//!   copy. Pins therefore also remember a fingerprint of the entry's preview.
//!
//! # Passwords
//!
//! Exclusion happens when the copy is recorded, not afterwards, because the database keeps no
//! clipboard hints. Password managers (1Password, KeePassXC, Bitwarden, ...) offer the
//! `x-kde-passwordManagerHint` MIME type with the value `secret`. `wl-paste --watch` (wl-clipboard
//! 2.2 and later; the installed one is 2.3.0) turns that into `CLIPBOARD_STATE=sensitive` in the
//! environment of the command it runs, and `clipvault store` refuses sensitive copies unless it
//! is given `--store-sensitive`, which Dojo's service does not pass. Nothing in this module can
//! add to that guarantee for copies made before it was in place, so as defence in depth
//! [`looks_secret`] masks previews of entries that look like private keys, well-known API tokens
//! or `password=...` assignments. A masked entry shows [`SECRET_PREVIEW`], is left out of
//! search, and can still be re-copied or deleted. Remaining gap: a secret copied from an
//! application that does not set the hint and matches none of those shapes is recorded and shown.
//! `clipvault store --ignore-pattern` and `--window-ignore-pattern` in the service's command
//! line would close it for chosen applications.
//!
//! # Limits
//!
//! Search sees each text entry's first 300 characters, which is what `clipvault list` returns;
//! reading every full text would cost a subprocess per entry. Binary and non-UTF-8 entries are
//! listed with their kind but kept out of search and out of the text list. Pins only outlive an
//! entry for as long as clipvault keeps it (14 days or 1000 entries by default).

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// Preview width asked of clipvault, in characters.
const LIST_PREVIEW_WIDTH: usize = 300;
/// Preview width shown to people.
const DISPLAY_CHARS: usize = 140;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const OUTPUT_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_PINS: usize = 200;
/// Shown in place of an entry that looks like a secret.
pub const SECRET_PREVIEW: &str = "Hidden: looks like a secret";
const BINARY_PREFIX: &str = "[[ binary data ";
const NON_UTF8_PREVIEW: &str = "[[ Non-UTF-8 text ]]";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Text,
    /// Images and other non-text data, or text that is not valid UTF-8.
    Binary,
}

/// One entry of clipvault's history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipEntry {
    /// Clipvault's id, which `get` and `delete` take.
    pub id: u64,
    pub kind: EntryKind,
    /// `text/plain` for text; the stored type (`image/png`) for binary data.
    pub mime: String,
    /// What a row shows: a single line cut to 140 characters, or [`SECRET_PREVIEW`], or a
    /// description such as `Image 26kB (image/png)`.
    pub preview: String,
    /// What search matches: the start of the text (up to 300 characters). Empty for binary
    /// entries and for entries that look like secrets.
    pub search_text: String,
    /// The text was longer than the preview.
    pub truncated: bool,
    /// The entry looks like a secret; see [`looks_secret`].
    pub sensitive: bool,
    /// Clipvault's rounded size, such as `26B` or `4kB`.
    pub size_label: String,
    /// When the content was last copied, in seconds since the Unix epoch (UTC).
    pub copied_at: u64,
    fingerprint: String,
}

// ---------------------------------------------------------------------------------------------
// Secrets and sanitising

const SECRET_KEYS: [&str; 12] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "api_key",
    "apikey",
    "api-key",
    "private_key",
    "access_key",
    "authorization",
    "credential",
];

/// Prefixes of well-known API credentials, with the least length a real one has.
const TOKEN_PREFIXES: [(&str, usize); 17] = [
    ("ghp_", 30),
    ("gho_", 30),
    ("ghu_", 30),
    ("ghs_", 30),
    ("ghr_", 30),
    ("github_pat_", 30),
    ("glpat-", 20),
    ("sk-", 20),
    ("sk_live_", 20),
    ("sk_test_", 20),
    ("rk_live_", 20),
    ("xoxb-", 20),
    ("xoxp-", 20),
    ("xapp-", 20),
    ("npm_", 30),
    ("hf_", 30),
    ("pypi-", 30),
];

fn is_base64url(text: &str) -> bool {
    text.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '='))
}

fn secret_key(word: &str) -> bool {
    let key = word
        .trim_matches(|c: char| matches!(c, '"' | '\'' | '-' | ':' | '=' | ','))
        .to_ascii_lowercase();
    !key.is_empty() && SECRET_KEYS.iter().any(|secret| key.contains(secret))
}

fn has_value(text: &str) -> bool {
    text.trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';'))
        .chars()
        .count()
        >= 4
}

/// Whether text has the shape of a credential: a private-key block, a single token with a
/// well-known API prefix (GitHub, GitLab, Stripe, Slack, npm, ...), an AWS access key id, a JWT,
/// or a `password=value`, `token: value` style assignment. Deliberately narrow: prose and
/// ordinary words never match, so little is hidden by mistake. It is a second line of defence
/// behind the hint-based exclusion described in the module documentation.
pub fn looks_secret(text: &str) -> bool {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return true;
    }
    let trimmed = text.trim();
    if !trimmed.is_empty() && !trimmed.contains(char::is_whitespace) {
        if TOKEN_PREFIXES
            .iter()
            .any(|(prefix, least)| trimmed.starts_with(prefix) && trimmed.len() >= *least)
        {
            return true;
        }
        let aws = trimmed.len() == 20
            && (trimmed.starts_with("AKIA") || trimmed.starts_with("ASIA"))
            && trimmed
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
        if aws {
            return true;
        }
        let mut parts = trimmed.split('.');
        if let (Some(header), Some(body), Some(signature), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
            && header.starts_with("eyJ")
            && body.len() >= 8
            && [header, body, signature].into_iter().all(is_base64url)
        {
            return true;
        }
    }
    let words = text.split_whitespace().collect::<Vec<_>>();
    words.iter().enumerate().any(|(index, word)| {
        if let Some((key, value)) = word.split_once('=') {
            secret_key(key) && has_value(value)
        } else if let Some(key) = word.strip_suffix(':') {
            secret_key(key) && words.get(index + 1).is_some_and(|next| has_value(next))
        } else {
            false
        }
    })
}

/// Control characters become spaces, and characters that reorder or hide text (bidirectional
/// overrides, zero-width marks) are dropped, so a preview is one honest line.
fn sanitize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2069}'
            | '\u{feff}' => {}
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn cut(text: &str, chars: usize) -> (String, bool) {
    if text.chars().count() <= chars {
        return (text.to_owned(), false);
    }
    let mut shortened = text.chars().take(chars).collect::<String>();
    shortened.truncate(shortened.trim_end().len());
    shortened.push('…');
    (shortened, true)
}

// ---------------------------------------------------------------------------------------------
// Parsing

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (month + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Parses clipvault's `2026-10-24T05:32:07Z` into seconds since the epoch.
pub fn parse_timestamp(text: &str) -> Option<u64> {
    let text = text.trim();
    let (date, time) = text.split_once('T')?;
    let time = time.trim_end_matches('Z');
    let time = time.split(['.', '+']).next()?;
    let mut date = date.split('-');
    let (year, month, day) = (
        date.next()?.parse::<i64>().ok()?,
        date.next()?.parse::<i64>().ok()?,
        date.next()?.parse::<i64>().ok()?,
    );
    let mut time = time.split(':');
    let (hour, minute, second) = (
        time.next()?.parse::<i64>().ok()?,
        time.next()?.parse::<i64>().ok()?,
        time.next()?.parse::<i64>().ok()?,
    );
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    u64::try_from(seconds).ok()
}

fn valid_mime(mime: &str) -> bool {
    let Some((kind, subtype)) = mime.split_once('/') else {
        return false;
    };
    let part = |part: &str| {
        !part.is_empty()
            && part.len() <= 100
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '+' | '-' | '_'))
    };
    part(kind) && part(subtype) && !kind.starts_with('-')
}

fn fingerprint(preview: &str) -> String {
    let digest = Sha256::digest(preview.as_bytes());
    digest[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Splits one `list` line (`id, preview, size, last-updated`). The preview may itself contain
/// tabs if clipvault ever stops collapsing them, so it is whatever lies between the first and
/// the last two fields.
fn parse_line(line: &str) -> Option<ClipEntry> {
    let fields = line.split('\t').collect::<Vec<_>>();
    if fields.len() < 4 {
        return None;
    }
    let id = fields[0].trim().parse::<u64>().ok()?;
    let copied_at = parse_timestamp(fields[fields.len() - 1])?;
    let size_label = fields[fields.len() - 2].trim().to_owned();
    let raw = fields[1..fields.len() - 2].join(" ");
    let fingerprint = fingerprint(&raw);

    let binary_mime = raw.strip_prefix(BINARY_PREFIX).map(|rest| {
        let mut words = rest.split_whitespace().skip(1);
        words
            .next()
            .filter(|mime| valid_mime(mime))
            .unwrap_or("application/octet-stream")
            .to_owned()
    });
    if binary_mime.is_some() || raw == NON_UTF8_PREVIEW {
        let mime = binary_mime.unwrap_or_else(|| "application/octet-stream".to_owned());
        let label = if mime.starts_with("image/") {
            "Image"
        } else {
            "Binary data"
        };
        return Some(ClipEntry {
            id,
            kind: EntryKind::Binary,
            preview: format!("{label} {size_label} ({mime})"),
            mime,
            search_text: String::new(),
            truncated: false,
            sensitive: false,
            size_label,
            copied_at,
            fingerprint,
        });
    }

    let clean = sanitize(&raw);
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let sensitive = looks_secret(&clean);
    let (preview, search_text, truncated) = if sensitive {
        (SECRET_PREVIEW.to_owned(), String::new(), false)
    } else {
        let (shown, cut_short) = cut(&clean, DISPLAY_CHARS);
        // clipvault marks its own cut with a trailing ellipsis.
        let source_cut = clean.ends_with('…') && raw.chars().count() >= LIST_PREVIEW_WIDTH;
        (shown, clean.clone(), cut_short || source_cut)
    };
    Some(ClipEntry {
        id,
        kind: EntryKind::Text,
        mime: "text/plain".to_owned(),
        preview,
        search_text,
        truncated,
        sensitive,
        size_label,
        copied_at,
        fingerprint,
    })
}

/// Parses `clipvault list -f id,preview,size,last-updated` output, newest first. Lines that do
/// not parse are skipped.
pub fn parse_list(text: &str) -> Vec<ClipEntry> {
    let mut entries = text.lines().filter_map(parse_line).collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .copied_at
            .cmp(&left.copied_at)
            .then_with(|| right.id.cmp(&left.id))
    });
    entries
}

// ---------------------------------------------------------------------------------------------
// Pins

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
struct Pin {
    id: u64,
    fingerprint: String,
}

/// The entries the user pinned, kept in a small JSON file beside the launcher's other state.
/// A pin names the clipvault id and a fingerprint of the entry's preview, so an id that clipvault
/// reuses for different content does not inherit the pin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pins {
    pins: BTreeSet<Pin>,
}

impl Pins {
    pub fn is_pinned(&self, entry: &ClipEntry) -> bool {
        self.pins
            .iter()
            .any(|pin| pin.id == entry.id && pin.fingerprint == entry.fingerprint)
    }

    pub fn pin(&mut self, entry: &ClipEntry) {
        if self.is_pinned(entry) {
            return;
        }
        self.pins.retain(|pin| pin.id != entry.id);
        self.pins.insert(Pin {
            id: entry.id,
            fingerprint: entry.fingerprint.clone(),
        });
        while self.pins.len() > MAX_PINS {
            // The oldest ids go first; ids only grow, so these are the stalest pins.
            if let Some(first) = self.pins.iter().next().cloned() {
                self.pins.remove(&first);
            }
        }
    }

    pub fn unpin(&mut self, entry: &ClipEntry) {
        self.unpin_id(entry.id);
    }

    /// Drops the pin on an id, for after the entry is deleted.
    pub fn unpin_id(&mut self, id: u64) {
        self.pins.retain(|pin| pin.id != id);
    }

    /// Pins an unpinned entry, unpins a pinned one; returns whether it is pinned now.
    pub fn toggle(&mut self, entry: &ClipEntry) -> bool {
        if self.is_pinned(entry) {
            self.unpin(entry);
            false
        } else {
            self.pin(entry);
            true
        }
    }

    /// Forgets pins whose entry is gone or now holds different content. Call it with a complete,
    /// successful listing only.
    pub fn retain_existing(&mut self, entries: &[ClipEntry]) {
        self.pins.retain(|pin| {
            entries
                .iter()
                .any(|entry| entry.id == pin.id && entry.fingerprint == pin.fingerprint)
        });
    }

    pub fn len(&self) -> usize {
        self.pins.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    /// A missing or damaged file is no pins: a bad file never keeps the view from opening.
    pub fn load(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).unwrap_or_else(|error| {
                eprintln!(
                    "Dojo ignored the clipboard pins at {}: {error}",
                    path.display()
                );
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                eprintln!(
                    "Dojo could not read the clipboard pins at {}: {error}",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Writes through a temporary file in the same directory and renames it over the target, so
    /// a crash leaves the old file or the new one, never half of either.
    pub fn save(&self, path: &Path) -> Result<()> {
        let parent = path
            .parent()
            .context("clipboard pins path has no parent directory")?;
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
        let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let contents = serde_json::to_vec(self).context("serializing the clipboard pins")?;
        let mut file = fs::File::create(&temporary)
            .with_context(|| format!("creating {}", temporary.display()))?;
        file.write_all(&contents)
            .and_then(|()| file.sync_all())
            .with_context(|| format!("writing {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .inspect_err(|_| {
                let _ = fs::remove_file(&temporary);
            })
            .with_context(|| format!("replacing {} with {}", path.display(), temporary.display()))
    }
}

/// `$XDG_STATE_HOME/dojo/clipboard-pins.json`, beside the launch history.
pub fn default_pins_path() -> Result<PathBuf> {
    Ok(crate::launch_history::default_path()?.with_file_name("clipboard-pins.json"))
}

// ---------------------------------------------------------------------------------------------
// Search

fn newest_first(left: &ClipEntry, right: &ClipEntry) -> std::cmp::Ordering {
    right
        .copied_at
        .cmp(&left.copied_at)
        .then_with(|| right.id.cmp(&left.id))
}

/// Text entries to show for `query`, best first, as indices into `entries`.
///
/// An empty query lists every text entry: pinned ones first, then the rest, each newest first.
/// Otherwise an entry must match its text, and matches fall into tiers: the whole text equals the
/// query; the text starts with it; it appears inside the text; its characters appear in order
/// (fuzzy). Within a tier pinned entries come first; the exact, prefix and substring tiers then
/// go newest first, and the fuzzy tier by match score and then newest. Entries that look like
/// secrets are listed for an empty query but never match a typed one. Binary entries never
/// appear.
pub fn search(
    entries: &[ClipEntry],
    pins: &Pins,
    query: &str,
    matcher: &mut Matcher,
) -> Vec<usize> {
    let query = query.trim();
    let text_entries = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.kind == EntryKind::Text);
    if query.is_empty() {
        let mut all = text_entries.map(|(index, _)| index).collect::<Vec<_>>();
        all.sort_by(|&left, &right| {
            pins.is_pinned(&entries[right])
                .cmp(&pins.is_pinned(&entries[left]))
                .then_with(|| newest_first(&entries[left], &entries[right]))
        });
        return all;
    }
    let atom = |kind| {
        Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            kind,
            false,
        )
    };
    let exact = atom(AtomKind::Exact);
    let prefix = atom(AtomKind::Prefix);
    let substring = atom(AtomKind::Substring);
    let fuzzy = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );

    let mut matched = Vec::new();
    for (index, entry) in text_entries {
        if entry.sensitive || entry.search_text.is_empty() {
            continue;
        }
        let text = Utf32String::from(entry.search_text.as_str());
        let tier_and_score = if exact.score(text.slice(..), matcher).is_some() {
            (0, 0)
        } else if prefix.score(text.slice(..), matcher).is_some() {
            (1, 0)
        } else if substring.score(text.slice(..), matcher).is_some() {
            (2, 0)
        } else if let Some(score) = fuzzy.score(text.slice(..), matcher) {
            (3, score)
        } else {
            continue;
        };
        matched.push((tier_and_score, index));
    }
    matched.sort_by(
        |&((left_tier, left_score), left), &((right_tier, right_score), right)| {
            left_tier
                .cmp(&right_tier)
                .then_with(|| {
                    pins.is_pinned(&entries[right])
                        .cmp(&pins.is_pinned(&entries[left]))
                })
                .then_with(|| right_score.cmp(&left_score))
                .then_with(|| newest_first(&entries[left], &entries[right]))
        },
    );
    matched.into_iter().map(|(_, index)| index).collect()
}

// ---------------------------------------------------------------------------------------------
// Commands

/// Where the clipvault and wl-copy programs are and which database to read. The default finds
/// both on `PATH` and uses clipvault's own default database.
#[derive(Clone, Debug)]
pub struct Clipvault {
    clipvault: PathBuf,
    wl_copy: PathBuf,
    database: Option<PathBuf>,
}

impl Default for Clipvault {
    fn default() -> Self {
        Self::new()
    }
}

impl Clipvault {
    pub fn new() -> Self {
        Self {
            clipvault: PathBuf::from("clipvault"),
            wl_copy: PathBuf::from("wl-copy"),
            database: None,
        }
    }

    pub fn with_database(mut self, database: impl Into<PathBuf>) -> Self {
        self.database = Some(database.into());
        self
    }

    pub fn with_programs(
        mut self,
        clipvault: impl Into<PathBuf>,
        wl_copy: impl Into<PathBuf>,
    ) -> Self {
        self.clipvault = clipvault.into();
        self.wl_copy = wl_copy.into();
        self
    }

    fn database_args(&self) -> Vec<String> {
        self.database
            .iter()
            .flat_map(|path| ["-d".to_owned(), path.to_string_lossy().into_owned()])
            .collect()
    }

    /// `clipvault list` with the fields and width [`parse_list`] expects.
    pub fn list_args(&self) -> Vec<String> {
        let mut args = [
            "list",
            "-m",
            &LIST_PREVIEW_WIDTH.to_string(),
            "-f",
            "id,preview,size,last-updated",
        ]
        .map(str::to_owned)
        .to_vec();
        args.extend(self.database_args());
        args
    }

    /// `clipvault get <id>`: the entry's bytes on standard output.
    pub fn get_args(&self, id: u64) -> Vec<String> {
        let mut args = vec!["get".to_owned(), id.to_string()];
        args.extend(self.database_args());
        args
    }

    /// `clipvault delete <id>`.
    pub fn delete_args(&self, id: u64) -> Vec<String> {
        let mut args = vec!["delete".to_owned(), id.to_string()];
        args.extend(self.database_args());
        args
    }

    /// The `wl-copy` command that puts an entry's bytes on the clipboard: text as `text/plain`
    /// (which wl-copy also offers as `UTF8_STRING` and the other text aliases) and other data
    /// under its own type. The bytes go to its standard input.
    pub fn copy_args(&self, entry: &ClipEntry) -> Vec<String> {
        let mime = match entry.kind {
            EntryKind::Text => "text/plain",
            EntryKind::Binary => entry.mime.as_str(),
        };
        vec!["--type".to_owned(), mime.to_owned()]
    }

    /// Every entry, newest first, with secrets masked.
    pub fn list(&self) -> Result<Vec<ClipEntry>> {
        let output = run(
            &self.clipvault,
            &self.list_args(),
            None,
            true,
            COMMAND_TIMEOUT,
        )?;
        if !output.success {
            bail!("clipvault list failed: {}", output.error_text());
        }
        Ok(parse_list(&String::from_utf8_lossy(&output.stdout)))
    }

    /// The entry's full content as text. Invalid UTF-8 is replaced. For a masked entry the real
    /// text is returned: this is what a re-copy sends, and the caller decides about showing it.
    pub fn read_text(&self, id: u64) -> Result<String> {
        Ok(String::from_utf8_lossy(&self.read_bytes(id)?).into_owned())
    }

    fn read_bytes(&self, id: u64) -> Result<Vec<u8>> {
        let output = run(
            &self.clipvault,
            &self.get_args(id),
            None,
            true,
            COMMAND_TIMEOUT,
        )?;
        if !output.success {
            bail!(
                "clipvault could not read entry {id}: {}",
                output.error_text()
            );
        }
        if output.stdout.is_empty() {
            bail!("clipvault entry {id} is empty");
        }
        Ok(output.stdout)
    }

    /// Puts the entry on the clipboard. The content is read completely before `wl-copy` starts,
    /// so a missing entry can never replace the clipboard with nothing. Clipvault's watcher then
    /// records the copy, which moves the entry to the top of the history.
    pub fn copy(&self, entry: &ClipEntry) -> Result<()> {
        let bytes = self.read_bytes(entry.id)?;
        // wl-copy keeps serving the selection from a background process. Its output is not
        // captured so that process cannot hold our pipes open.
        let output = run(
            &self.wl_copy,
            &self.copy_args(entry),
            Some(bytes),
            false,
            COMMAND_TIMEOUT,
        )?;
        if output.success {
            Ok(())
        } else {
            bail!("wl-copy failed to take entry {}", entry.id)
        }
    }

    /// Deletes one entry from clipvault's history. Remove its pin too.
    pub fn delete(&self, id: u64) -> Result<()> {
        let output = run(
            &self.clipvault,
            &self.delete_args(id),
            None,
            true,
            COMMAND_TIMEOUT,
        )?;
        if output.success {
            Ok(())
        } else {
            bail!(
                "clipvault could not delete entry {id}: {}",
                output.error_text()
            )
        }
    }
}

struct Output {
    success: bool,
    stdout: Vec<u8>,
    stderr: String,
}

impl Output {
    fn error_text(&self) -> String {
        let text = sanitize(self.stderr.trim());
        if text.is_empty() {
            "no error text".to_owned()
        } else {
            cut(&text, 300).0
        }
    }
}

/// Runs a program with a deadline, optionally feeding its standard input. With `capture` false
/// the output streams are discarded rather than read.
fn run(
    program: &Path,
    args: &[String],
    stdin: Option<Vec<u8>>,
    capture: bool,
    timeout: Duration,
) -> Result<Output> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(if capture {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    let writer = stdin.and_then(|bytes| {
        let mut pipe = child.stdin.take()?;
        // A thread feeds the pipe so a large entry cannot deadlock against the child's output.
        Some(std::thread::spawn(move || {
            let _ = pipe.write_all(&bytes);
        }))
    });
    let reader = |stream: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(OUTPUT_LIMIT).read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = reader(child.stdout.take().map(|s| Box::new(s) as _));
    let stderr = reader(child.stderr.take().map(|s| Box::new(s) as _));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for the command")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{} did not finish within {} seconds",
                program.display(),
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    if let Some(writer) = writer {
        let _ = writer.join();
    }
    Ok(Output {
        success: status.success(),
        stdout: stdout.join().unwrap_or_default(),
        stderr: String::from_utf8_lossy(&stderr.join().unwrap_or_default()).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt as _;

    // Shaped like real `clipvault list -m 300 -f id,preview,size,last-updated` output.
    fn list_text() -> String {
        [
            "7\thttps://example.org/path\t26B\t2026-10-24T05:32:07Z",
            "6\t# Handoff: T3 Code single-story `a` ## Goal User wants\t4kB\t2026-10-24T04:17:41Z",
            "5\t[[ binary data 26kB image/png ]]\t26kB\t2026-10-24T03:49:43Z",
            "4\t[[ Non-UTF-8 text ]]\t6B\t2026-10-24T03:00:00Z",
            "3\tghp_abcdefghijklmnopqrstuvwxyz0123456789\t41B\t2026-10-23T12:00:00Z",
            "2\thello world\t11B\t2026-10-23T11:00:00Z",
            "1\thello there\t11B\t2026-10-22T11:00:00Z",
        ]
        .join("\n")
    }

    fn entries() -> Vec<ClipEntry> {
        parse_list(&list_text())
    }

    fn entry(id: u64, text: &str, at: u64) -> ClipEntry {
        parse_line(&format!("{id}\t{text}\t10B\t{}", iso(at))).unwrap()
    }

    fn iso(seconds: u64) -> String {
        // Only used with small offsets inside 2026-10-24.
        let hour = seconds / 3600;
        let minute = seconds % 3600 / 60;
        let second = seconds % 60;
        format!("2026-10-24T{hour:02}:{minute:02}:{second:02}Z")
    }

    fn matcher() -> Matcher {
        Matcher::new(nucleo_matcher::Config::DEFAULT)
    }

    fn found(entries: &[ClipEntry], pins: &Pins, query: &str) -> Vec<u64> {
        search(entries, pins, query, &mut matcher())
            .into_iter()
            .map(|index| entries[index].id)
            .collect()
    }

    #[test]
    fn timestamps_parse_to_epoch_seconds() {
        assert_eq!(parse_timestamp("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_timestamp("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_timestamp("2026-10-24T05:32:07Z"), Some(1_792_819_927));
        // The leap day, and a fraction that systemd-style sources might add.
        assert_eq!(parse_timestamp("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        assert_eq!(
            parse_timestamp("2026-10-24T05:32:07.123Z"),
            Some(1_792_819_927)
        );
        assert_eq!(parse_timestamp("2026-13-24T05:32:07Z"), None);
        assert_eq!(parse_timestamp("yesterday"), None);
        assert_eq!(parse_timestamp(""), None);
    }

    #[test]
    fn the_list_parses_newest_first_with_kinds_sizes_and_types() {
        let entries = entries();
        assert_eq!(
            entries.iter().map(|e| e.id).collect::<Vec<_>>(),
            [7, 6, 5, 4, 3, 2, 1]
        );
        assert_eq!(entries[0].preview, "https://example.org/path");
        assert_eq!(entries[0].mime, "text/plain");
        assert_eq!(entries[0].size_label, "26B");
        assert_eq!(entries[0].copied_at, 1_792_819_927);
        let image = &entries[2];
        assert_eq!(image.kind, EntryKind::Binary);
        assert_eq!(image.mime, "image/png");
        assert_eq!(image.preview, "Image 26kB (image/png)");
        assert!(image.search_text.is_empty());
        let odd = &entries[3];
        assert_eq!(odd.kind, EntryKind::Binary);
        assert_eq!(odd.mime, "application/octet-stream");
    }

    #[test]
    fn unsorted_or_damaged_lines_are_ordered_and_skipped() {
        let text = "1\told\t1B\t2026-10-24T00:00:10Z\n\
                    garbage without tabs\n\
                    x\tbad id\t1B\t2026-10-24T00:00:10Z\n\
                    2\tbad time\t1B\tnow\n\
                    3\tnew\t1B\t2026-10-24T00:00:20Z\n\
                    4\tsame time higher id\t1B\t2026-10-24T00:00:20Z\n";
        assert_eq!(
            parse_list(text).iter().map(|e| e.id).collect::<Vec<_>>(),
            [4, 3, 1]
        );
        assert!(parse_list("").is_empty());
    }

    #[test]
    fn previews_are_one_clean_bounded_line() {
        let long = "word ".repeat(200);
        let item = entry(1, &long, 100);
        assert!(item.preview.chars().count() <= DISPLAY_CHARS + 1);
        assert!(item.preview.ends_with('…'));
        assert!(item.truncated);
        assert!(item.search_text.len() > item.preview.len());

        let messy = entry(2, "a\u{1b}[31mred\u{202e}rtl\u{200b}hidden  \u{7}bell", 100);
        assert_eq!(messy.preview, "a [31mredrtlhidden bell");

        let short = entry(3, "short", 100);
        assert!(!short.truncated);
        let from_clipvault = entry(4, &format!("{}…", "y".repeat(299)), 100);
        assert!(
            from_clipvault.truncated,
            "clipvault's own cut counts as truncated"
        );
    }

    #[test]
    fn a_hostile_mime_in_a_binary_preview_is_not_trusted() {
        let hostile =
            parse_line("1\t[[ binary data 5kB --type=evil ]]\t5kB\t2026-10-24T00:00:00Z").unwrap();
        assert_eq!(hostile.mime, "application/octet-stream");
        let odd =
            parse_line("1\t[[ binary data 5kB image/png ]]\t5kB\t2026-10-24T00:00:00Z").unwrap();
        assert_eq!(odd.mime, "image/png");
        assert!(valid_mime("application/vnd.api+json"));
        assert!(!valid_mime("image"));
        assert!(!valid_mime("-x/y"));
        assert!(!valid_mime("a/b c"));
    }

    #[test]
    fn secrets_are_recognised_by_shape_and_prose_is_left_alone() {
        for secret in [
            "-----BEGIN OPENSSH PRIVATE KEY----- b3BlbnNzaC1rZXk",
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
            "github_pat_11ABCDEFG0abcdefghijklmnopqrstuvwxyz",
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            "xoxb-1234567890-abcdefghij",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dBjftJeZ4CVPmB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "PASSWORD=hunter2hunter2",
            "export API_KEY=abc123def456",
            "db_password: correct-horse-battery",
            "curl -H Authorization: Bearer abcdefgh1234",
            "token=\"abcd1234efgh\"",
        ] {
            assert!(looks_secret(secret), "should be secret: {secret}");
        }
        for fine in [
            "https://example.org/some/path?query=1",
            "hello world",
            "the password is hard to remember",
            "password:",
            "token=",
            "git commit -m 'fix token refresh'",
            "sk-1",
            "AKIAshort",
            "a.b.c",
            "ghp_short",
            "",
            "tokenizer = Tokenizer()",
        ] {
            assert!(!looks_secret(fine), "should not be secret: {fine}");
        }
    }

    #[test]
    fn secret_looking_entries_never_show_their_text() {
        let entries = entries();
        let secret = entries.iter().find(|e| e.id == 3).unwrap();
        assert!(secret.sensitive);
        assert_eq!(secret.preview, SECRET_PREVIEW);
        assert!(secret.search_text.is_empty());
        assert!(!secret.preview.contains("ghp_"));
        // And it cannot be found by typing part of it.
        assert!(!found(&entries, &Pins::default(), "ghp_abcdef").contains(&3));
        assert!(!found(&entries, &Pins::default(), "abcdefghijklmnop").contains(&3));
        // But it stays reachable in the plain list so it can be re-copied or deleted.
        assert!(found(&entries, &Pins::default(), "").contains(&3));
    }

    #[test]
    fn an_empty_query_lists_text_newest_first_with_pins_on_top() {
        let entries = entries();
        let mut pins = Pins::default();
        assert_eq!(
            found(&entries, &pins, ""),
            [7, 6, 3, 2, 1],
            "binary entries are left out"
        );
        pins.pin(entries.iter().find(|e| e.id == 1).unwrap());
        pins.pin(entries.iter().find(|e| e.id == 2).unwrap());
        assert_eq!(found(&entries, &pins, "  "), [2, 1, 7, 6, 3]);
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_substring_then_fuzzy() {
        let entries = vec![
            entry(1, "say hello", 10),
            entry(2, "hello world", 20),
            entry(3, "hello", 5),
            entry(4, "h e l l o spread", 30),
            entry(5, "unrelated", 40),
            entry(6, "HELLO shouting", 15),
        ];
        let pins = Pins::default();
        // Exact first; prefixes by recency (the shouting one is older than "hello world");
        // then the substring match; the spread-out fuzzy match last.
        assert_eq!(found(&entries, &pins, "hello"), [3, 2, 6, 1, 4]);
        assert_eq!(found(&entries, &pins, "zzz"), Vec::<u64>::new());
        // Smart case: a query with a capital matches case-sensitively.
        assert_eq!(found(&entries, &pins, "HELLO"), [6]);
    }

    #[test]
    fn ties_go_to_the_newest_and_pins_beat_recency_within_a_tier() {
        let entries = vec![
            entry(1, "note one", 10),
            entry(2, "note two", 30),
            entry(3, "note three", 20),
        ];
        let mut pins = Pins::default();
        assert_eq!(found(&entries, &pins, "note"), [2, 3, 1]);
        pins.pin(&entries[0]);
        assert_eq!(found(&entries, &pins, "note"), [1, 2, 3]);
    }

    #[test]
    fn pins_toggle_survive_a_round_trip_and_ignore_reused_ids() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("state/dojo/clipboard-pins.json");
        assert_eq!(Pins::load(&path), Pins::default());

        let original = entry(5, "keep me", 10);
        let mut pins = Pins::default();
        assert!(pins.toggle(&original));
        assert!(pins.is_pinned(&original));
        pins.save(&path)?;
        assert_eq!(Pins::load(&path), pins);
        assert!(
            !fs::read_dir(path.parent().unwrap())?
                .any(|e| { e.unwrap().file_name().to_string_lossy().ends_with(".tmp") })
        );

        // Clipvault hands id 5 to different content after the old entry was deleted.
        let reused = entry(5, "something else", 99);
        assert!(!pins.is_pinned(&reused));
        pins.retain_existing(&[reused.clone()]);
        assert!(pins.is_empty());

        assert!(pins.toggle(&original));
        assert!(!pins.toggle(&original));
        assert!(pins.is_empty());

        fs::write(&path, "{ not json")?;
        assert_eq!(Pins::load(&path), Pins::default());
        Ok(())
    }

    #[test]
    fn retaining_keeps_pins_for_entries_still_present_and_unpin_id_removes() {
        let kept = entry(1, "a", 1);
        let gone = entry(2, "b", 2);
        let mut pins = Pins::default();
        pins.pin(&kept);
        pins.pin(&gone);
        pins.retain_existing(&[kept.clone()]);
        assert_eq!(pins.len(), 1);
        pins.unpin_id(1);
        assert!(pins.is_empty());
    }

    #[test]
    fn the_pin_count_is_capped() {
        let mut pins = Pins::default();
        for id in 0..(MAX_PINS as u64 + 20) {
            pins.pin(&entry(id, &format!("entry {id}"), 1));
        }
        assert_eq!(pins.len(), MAX_PINS);
        assert!(pins.is_pinned(&entry(
            MAX_PINS as u64 + 19,
            &format!("entry {}", MAX_PINS + 19),
            1
        )));
    }

    #[test]
    fn argv_for_each_command_is_built_from_numbers_not_text() {
        let plain = Clipvault::new();
        assert_eq!(
            plain.list_args(),
            ["list", "-m", "300", "-f", "id,preview,size,last-updated"]
        );
        assert_eq!(plain.get_args(42), ["get", "42"]);
        assert_eq!(plain.delete_args(42), ["delete", "42"]);
        let custom = Clipvault::new().with_database("/tmp/x.db");
        assert_eq!(custom.get_args(1), ["get", "1", "-d", "/tmp/x.db"]);
        assert_eq!(custom.list_args().last().unwrap(), "/tmp/x.db");

        let entries = entries();
        assert_eq!(plain.copy_args(&entries[0]), ["--type", "text/plain"]);
        assert_eq!(plain.copy_args(&entries[2]), ["--type", "image/png"]);
    }

    /// A stand-in for wl-copy that records its arguments and standard input.
    fn fake_wl_copy(directory: &Path) -> (PathBuf, PathBuf, PathBuf) {
        let script = directory.join("wl-copy");
        let arguments = directory.join("wl-copy.args");
        let input = directory.join("wl-copy.stdin");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\necho \"$@\" > '{}'\ncat > '{}'\n",
                arguments.display(),
                input.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        (script, arguments, input)
    }

    fn clipvault_available() -> bool {
        Command::new("clipvault")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn store(database: &Path, bytes: &[u8], sensitive: bool) {
        let mut command = Command::new("clipvault");
        command
            .args(["store", "-d"])
            .arg(database)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if sensitive {
            command.env("CLIPBOARD_STATE", "sensitive");
        }
        let mut child = command.spawn().unwrap();
        child.stdin.take().unwrap().write_all(bytes).unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn against_a_scratch_database_list_get_copy_and_delete_work() {
        if !clipvault_available() {
            eprintln!("skipped: clipvault is not installed");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("scratch.db");
        store(&database, b"first entry", false);
        store(&database, b"second\nentry\twith tab", false);
        store(&database, b"hunter2-from-a-password-manager", true);
        store(
            &database,
            b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89",
            false,
        );
        store(&database, b"\xff\xfe\x00bad utf8", false);

        let (wl_copy, arguments, input) = fake_wl_copy(directory.path());
        let vault = Clipvault::new()
            .with_database(&database)
            .with_programs("clipvault", &wl_copy);
        let listed = vault.list().unwrap();
        // The sensitive copy was never recorded: that is the guarantee the service relies on.
        assert_eq!(listed.len(), 4, "{listed:?}");
        let text = listed
            .iter()
            .find(|e| e.preview.starts_with("second"))
            .unwrap();
        assert_eq!(text.preview, "second entry with tab");
        assert_eq!(vault.read_text(text.id).unwrap(), "second\nentry\twith tab");
        let image = listed.iter().find(|e| e.mime == "image/png").unwrap();
        assert_eq!(image.kind, EntryKind::Binary);
        assert!(
            listed
                .iter()
                .any(|e| e.kind == EntryKind::Binary && e.mime == "application/octet-stream")
        );
        assert!(listed.iter().all(|e| !e.preview.contains("hunter2")));
        let found = search(&listed, &Pins::default(), "entry", &mut matcher());
        assert_eq!(found.len(), 2);

        // Copy feeds the stored bytes to wl-copy with the right type.
        vault.copy(text).unwrap();
        assert_eq!(
            fs::read_to_string(&arguments).unwrap().trim(),
            "--type text/plain"
        );
        assert_eq!(fs::read(&input).unwrap(), b"second\nentry\twith tab");
        vault.copy(image).unwrap();
        assert_eq!(
            fs::read_to_string(&arguments).unwrap().trim(),
            "--type image/png"
        );
        assert_eq!(fs::read(&input).unwrap().len(), 33);

        // Copying a vanished entry fails before wl-copy runs, leaving the clipboard alone.
        fs::remove_file(&arguments).unwrap();
        assert!(vault.copy(&entry(9999, "ghost", 1)).is_err());
        assert!(!arguments.exists());

        vault.delete(text.id).unwrap();
        assert_eq!(vault.list().unwrap().len(), 3);
        assert!(vault.delete(text.id).is_err());
        assert!(vault.read_text(text.id).is_err());
    }

    #[test]
    fn a_failing_wl_copy_and_a_missing_program_are_reported() {
        if !clipvault_available() {
            eprintln!("skipped: clipvault is not installed");
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("scratch.db");
        store(&database, b"content", false);
        let failing = directory.path().join("failing-wl-copy");
        fs::write(&failing, "#!/bin/sh\ncat >/dev/null\nexit 1\n").unwrap();
        fs::set_permissions(&failing, fs::Permissions::from_mode(0o755)).unwrap();

        let vault = Clipvault::new()
            .with_database(&database)
            .with_programs("clipvault", &failing);
        let listed = vault.list().unwrap();
        assert!(vault.copy(&listed[0]).is_err());
        let missing = Clipvault::new()
            .with_database(&database)
            .with_programs("clipvault", "/nonexistent/wl-copy");
        assert!(missing.copy(&listed[0]).is_err());
        let no_clipvault = Clipvault::new().with_programs("/nonexistent/clipvault", "wl-copy");
        assert!(no_clipvault.list().is_err());
    }

    #[test]
    fn run_enforces_its_deadline_and_feeds_large_input() {
        let slow = run(
            Path::new("sleep"),
            &["30".to_owned()],
            None,
            true,
            Duration::from_millis(100),
        );
        assert!(slow.err().unwrap().to_string().contains("did not finish"));
        let big = vec![b'x'; 3 * 1024 * 1024];
        let echoed = run(
            Path::new("cat"),
            &[],
            Some(big.clone()),
            true,
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(echoed.success);
        assert_eq!(echoed.stdout.len(), big.len());
    }

    /// Read-only: lists the real history when clipvault and its database exist. Prints counts
    /// only, never content.
    #[test]
    fn live_history_smoke() {
        if !clipvault_available() {
            eprintln!("skipped: clipvault is not installed");
            return;
        }
        let Ok(listed) = Clipvault::new().list() else {
            eprintln!("skipped: clipvault could not read its default database");
            return;
        };
        let texts = listed.iter().filter(|e| e.kind == EntryKind::Text).count();
        let masked = listed.iter().filter(|e| e.sensitive).count();
        eprintln!(
            "live history: {} entries, {texts} text, {masked} masked",
            listed.len()
        );
        assert!(
            listed
                .windows(2)
                .all(|pair| pair[0].copied_at >= pair[1].copied_at)
        );
        assert!(
            listed
                .iter()
                .all(|e| e.preview.chars().count() <= DISPLAY_CHARS + 40)
        );
        let order = search(&listed, &Pins::default(), "", &mut matcher());
        assert_eq!(order.len(), texts);
    }
}
