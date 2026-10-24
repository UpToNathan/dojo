//! 06 Snippets store and placeholders. See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! Snippets live in `$XDG_CONFIG_HOME/dojo/snippets.toml` (fallback `~/.config/dojo/snippets.toml`):
//!
//! ```toml
//! [[snippet]]
//! name = "Sign-off"
//! keywords = ["sig", "regards"]   # optional
//! text = """
//! Kind regards,
//! Nathan — {date}
//! """
//! ```
//!
//! A missing file is an empty list ([`LoadedSnippets::empty_hint`] tells the launcher what to show);
//! a malformed one is an error and never a panic. Search is smart-case (a capital in the query makes
//! it case-sensitive) and ranks exact name, exact keyword, name
//! prefix, keyword prefix, fuzzy name, fuzzy keyword, then file order. Expansion replaces
//! `{date}` (`YYYY-MM-DD`), `{time}` (`HH:MM`, 24 hour), `{datetime}` (`YYYY-MM-DD HH:MM`),
//! `{clipboard}` and `{uuid}` (a random v4 UUID, new for every occurrence). `{{` and `}}` are
//! literal braces. Placeholders are lowercase and exact; anything else, such as `{name}` or a lone
//! brace, stays as typed. Inserted values are not scanned again, so a clipboard that contains
//! `{date}` is copied as is.

use std::fs::{self, OpenOptions};
use std::io::{ErrorKind, Write as _};
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context as _, Result, bail};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};
use serde::{Deserialize, Serialize};

pub const RESULT_LIMIT: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snippet {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keywords: Vec<String>,
    pub text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnippetFile {
    #[serde(default)]
    snippet: Vec<Snippet>,
}

/// The snippet list as the launcher shows it: what was loaded, and why the list may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadedSnippets {
    pub snippets: Vec<Snippet>,
    /// Where the file is (or would be); `None` when the config location cannot be determined.
    pub path: Option<PathBuf>,
    /// Present when the file exists but could not be used; `snippets` is then empty.
    pub error: Option<String>,
}

impl LoadedSnippets {
    /// Load the user's file at the default location. Never fails: problems land in `error`.
    pub fn for_user() -> Self {
        match user_config_path() {
            Ok(path) => Self::from_path(&path),
            Err(error) => Self {
                snippets: Vec::new(),
                path: None,
                error: Some(format!("{error:#}")),
            },
        }
    }

    pub fn from_path(path: &Path) -> Self {
        match load(path) {
            Ok(snippets) => Self {
                snippets,
                path: Some(path.to_owned()),
                error: None,
            },
            Err(error) => Self {
                snippets: Vec::new(),
                path: Some(path.to_owned()),
                error: Some(format!("{error:#}")),
            },
        }
    }

    /// Text for the empty state: how to add snippets, or what is wrong with the file. `None` when
    /// there is something to show.
    pub fn empty_hint(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("Snippets file could not be read: {error}"));
        }
        if !self.snippets.is_empty() {
            return None;
        }
        Some(match &self.path {
            Some(path) => format!("No snippets yet. Add some to {}", path.display()),
            None => "No snippets yet.".to_owned(),
        })
    }
}

/// `$XDG_CONFIG_HOME/dojo/snippets.toml`, falling back to `~/.config/dojo/snippets.toml`.
pub fn user_config_path() -> Result<PathBuf> {
    let config_home = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => std::env::var_os("HOME")
            .map(PathBuf::from)
            .context("neither XDG_CONFIG_HOME nor HOME is set")?
            .join(".config"),
    };
    if !config_home.is_absolute() {
        bail!("XDG_CONFIG_HOME must be absolute");
    }
    Ok(config_home.join("dojo/snippets.toml"))
}

/// Read the snippets file. A missing file is an empty list; a file that cannot be read, is not
/// valid TOML, has unknown keys, or has a snippet without a name or text is an error.
pub fn load(path: &Path) -> Result<Vec<Snippet>> {
    match fs::read_to_string(path) {
        Ok(text) => parse(&text).with_context(|| format!("in {}", path.display())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// Parse the text of a snippets file.
pub fn parse(text: &str) -> Result<Vec<Snippet>> {
    let mut file: SnippetFile = toml::from_str(text).context("parsing snippets")?;
    for (index, snippet) in file.snippet.iter_mut().enumerate() {
        snippet.name = snippet.name.trim().to_owned();
        if snippet.name.is_empty() {
            bail!("snippet {} has an empty name", index + 1);
        }
        if snippet.text.is_empty() {
            bail!("snippet `{}` has no text", snippet.name);
        }
        snippet.keywords = std::mem::take(&mut snippet.keywords)
            .into_iter()
            .map(|keyword| keyword.trim().to_owned())
            .filter(|keyword| !keyword.is_empty())
            .collect();
    }
    Ok(file.snippet)
}

/// Add `snippet` to the end of the file at `path`, creating the file and its directory when
/// needed. The existing text, comments included, is kept as it is. A file that does not parse is
/// left untouched and reported, so a typo is never overwritten. The write is atomic.
pub fn append(path: &Path, snippet: &Snippet) -> Result<()> {
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => String::new(),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let known = parse(&existing).with_context(|| format!("in {}", path.display()))?;
    let added = serialize(std::slice::from_ref(snippet))?;

    let mut combined = existing;
    if !combined.is_empty() {
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
        combined.push('\n');
    }
    combined.push_str(&added);
    let parsed = parse(&combined).context("the new snippet does not form a valid file")?;
    if parsed.len() != known.len() + 1 {
        bail!("the new snippet was not added as a separate entry");
    }
    write_atomic(path, &combined)
}

/// Replace the whole file with `snippets`. Comments in the old file are lost.
pub fn save_all(path: &Path, snippets: &[Snippet]) -> Result<()> {
    let text = serialize(snippets)?;
    parse(&text).context("the snippets do not form a valid file")?;
    write_atomic(path, &text)
}

fn serialize(snippets: &[Snippet]) -> Result<String> {
    for snippet in snippets {
        if snippet.name.trim().is_empty() {
            bail!("a snippet needs a name");
        }
        if snippet.text.is_empty() {
            bail!("snippet `{}` needs text", snippet.name);
        }
    }
    toml::to_string(&SnippetFile {
        snippet: snippets.to_vec(),
    })
    .context("serializing snippets")
}

static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `contents` next to `path` and rename it into place, so readers see the old file or the
/// new one and never half of either. The file is private to the user (snippets may hold secrets)
/// unless the existing file had other permissions, which are kept.
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("snippets path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let file_name = path
        .file_name()
        .context("snippets path has no file name")?
        .to_string_lossy();
    let temporary_path = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let mode = fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o7777)
        .unwrap_or(0o600);

    let written = (|| -> Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(mode)
            .open(&temporary_path)
            .with_context(|| format!("creating {}", temporary_path.display()))?;
        file.write_all(contents.as_bytes())
            .with_context(|| format!("writing {}", temporary_path.display()))?;
        file.sync_all()
            .with_context(|| format!("syncing {}", temporary_path.display()))?;
        fs::rename(&temporary_path, path).with_context(|| {
            format!(
                "replacing {} with {}",
                path.display(),
                temporary_path.display()
            )
        })
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    written
}

/// Snippets matching `query`, best first, as indexes into `snippets`. An empty query lists all of
/// them in file order. Ties within a tier go to the better fuzzy score, then file order.
pub fn search(snippets: &[Snippet], query: &str, matcher: &mut Matcher) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..snippets.len().min(RESULT_LIMIT)).collect();
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
    let fuzzy = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );

    let mut scored = Vec::new();
    for (index, snippet) in snippets.iter().enumerate() {
        // Tier is 2 * match kind (exact, prefix, fuzzy) + 1 for a keyword, so a name match
        // outranks a keyword match of the same kind and any better kind outranks both.
        let mut best: Option<(u8, u32)> = None;
        let haystacks = std::iter::once((snippet.name.as_str(), 0))
            .chain(snippet.keywords.iter().map(|keyword| (keyword.as_str(), 1)));
        for (text, keyword) in haystacks {
            let haystack = Utf32String::from(text);
            let Some(score) = fuzzy.score(haystack.slice(..), matcher) else {
                continue;
            };
            let kind = if exact.score(haystack.slice(..), matcher).is_some() {
                0
            } else if prefix.score(haystack.slice(..), matcher).is_some() {
                1
            } else {
                2
            };
            let candidate: (u8, u32) = (2 * kind + keyword, score);
            if best.is_none_or(|(tier, score)| {
                candidate.0 < tier || (candidate.0 == tier && candidate.1 > score)
            }) {
                best = Some(candidate);
            }
        }
        if let Some((tier, score)) = best {
            scored.push((index, tier, score));
        }
    }
    scored.sort_unstable_by(
        |(left, left_tier, left_score), (right, right_tier, right_score)| {
            left_tier
                .cmp(right_tier)
                .then_with(|| right_score.cmp(left_score))
                .then_with(|| left.cmp(right))
        },
    );
    scored.truncate(RESULT_LIMIT);
    scored.into_iter().map(|(index, ..)| index).collect()
}

/// Local calendar time as shown in snippets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

impl DateTime {
    pub fn date(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn time(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    pub fn datetime(&self) -> String {
        format!("{} {}", self.date(), self.time())
    }
}

/// Everything expansion needs from the outside world, so tests can pin it.
pub trait ExpandContext {
    /// The current local time.
    fn now(&self) -> DateTime;
    /// Text currently on the clipboard; empty when there is none.
    fn clipboard(&self) -> String;
    /// Sixteen random bytes, used for `{uuid}`. Called once per occurrence.
    fn random_bytes(&self) -> [u8; 16];
}

/// The real clock and randomness, with clipboard text the launcher read beforehand.
#[derive(Clone, Debug, Default)]
pub struct SystemContext {
    pub clipboard: String,
}

impl ExpandContext for SystemContext {
    fn now(&self) -> DateTime {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs() as libc::time_t);
        local_date_time(timestamp).unwrap_or(DateTime {
            year: 1970,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
        })
    }

    fn clipboard(&self) -> String {
        self.clipboard.clone()
    }

    fn random_bytes(&self) -> [u8; 16] {
        *uuid::Uuid::new_v4().as_bytes()
    }
}

fn local_date_time(timestamp: libc::time_t) -> Option<DateTime> {
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&timestamp, local.as_mut_ptr()) }.is_null() {
        return None;
    }
    let local = unsafe { local.assume_init() };
    Some(DateTime {
        year: local.tm_year + 1900,
        month: u8::try_from(local.tm_mon + 1).ok()?,
        day: u8::try_from(local.tm_mday).ok()?,
        hour: u8::try_from(local.tm_hour).ok()?,
        minute: u8::try_from(local.tm_min).ok()?,
    })
}

/// The text to copy for `snippet`.
pub fn expand(snippet: &Snippet, context: &dyn ExpandContext) -> String {
    expand_text(&snippet.text, context)
}

/// Expand placeholders in `text`; see the module documentation for the rules.
pub fn expand_text(text: &str, context: &dyn ExpandContext) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(['{', '}']) {
        output.push_str(&rest[..position]);
        rest = &rest[position..];
        if let Some(after) = rest.strip_prefix("{{") {
            output.push('{');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("}}") {
            output.push('}');
            rest = after;
        } else if rest.starts_with('{') {
            let value = rest.find('}').and_then(|close| {
                placeholder(&rest[1..close], context).map(|value| (value, close))
            });
            match value {
                Some((value, close)) => {
                    output.push_str(&value);
                    rest = &rest[close + 1..];
                }
                None => {
                    output.push('{');
                    rest = &rest[1..];
                }
            }
        } else {
            output.push('}');
            rest = &rest[1..];
        }
    }
    output.push_str(rest);
    output
}

fn placeholder(name: &str, context: &dyn ExpandContext) -> Option<String> {
    Some(match name {
        "date" => context.now().date(),
        "time" => context.now().time(),
        "datetime" => context.now().datetime(),
        "clipboard" => context.clipboard(),
        "uuid" => uuid::Builder::from_random_bytes(context.random_bytes())
            .into_uuid()
            .hyphenated()
            .to_string(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    struct Fixed {
        now: DateTime,
        clipboard: &'static str,
        counter: Cell<u8>,
    }

    impl Fixed {
        fn new() -> Self {
            Self {
                now: DateTime {
                    year: 2026,
                    month: 3,
                    day: 7,
                    hour: 9,
                    minute: 5,
                },
                clipboard: "copied text",
                counter: Cell::new(0),
            }
        }
    }

    impl ExpandContext for Fixed {
        fn now(&self) -> DateTime {
            self.now
        }
        fn clipboard(&self) -> String {
            self.clipboard.to_owned()
        }
        fn random_bytes(&self) -> [u8; 16] {
            self.counter.set(self.counter.get() + 1);
            [self.counter.get(); 16]
        }
    }

    fn snippet(name: &str, keywords: &[&str], text: &str) -> Snippet {
        Snippet {
            name: name.into(),
            keywords: keywords.iter().map(|k| (*k).into()).collect(),
            text: text.into(),
        }
    }

    fn expanded(text: &str) -> String {
        expand_text(text, &Fixed::new())
    }

    fn names(snippets: &[Snippet], query: &str) -> Vec<String> {
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        search(snippets, query, &mut matcher)
            .into_iter()
            .map(|index| snippets[index].name.clone())
            .collect()
    }

    #[test]
    fn parses_names_keywords_and_text() {
        let snippets = parse(
            r#"
[[snippet]]
name = "Sign-off"
keywords = ["sig", " regards ", ""]
text = """
Kind regards,
Nathan
"""

[[snippet]]
name = "  Address "
text = "1 Example Street"
"#,
        )
        .unwrap();
        assert_eq!(snippets.len(), 2);
        assert_eq!(snippets[0].name, "Sign-off");
        assert_eq!(snippets[0].keywords, ["sig", "regards"]);
        assert_eq!(snippets[0].text, "Kind regards,\nNathan\n");
        assert_eq!(snippets[1].name, "Address");
        assert!(snippets[1].keywords.is_empty());
    }

    #[test]
    fn parses_unicode() {
        let snippets = parse("[[snippet]]\nname = \"Grüße 🦀\"\nkeywords = [\"日本\"]\ntext = \"こんにちは — ünïcode\"\n")
            .unwrap();
        assert_eq!(snippets[0].name, "Grüße 🦀");
        assert_eq!(snippets[0].keywords, ["日本"]);
        assert_eq!(snippets[0].text, "こんにちは — ünïcode");
    }

    #[test]
    fn empty_file_is_an_empty_list() {
        assert!(parse("").unwrap().is_empty());
        assert!(parse("# only a comment\n").unwrap().is_empty());
    }

    #[test]
    fn malformed_files_are_errors() {
        for bad in [
            "not toml [[[",
            "[[snippet]]\nname = \"x\"\n",
            "[[snippet]]\ntext = \"x\"\n",
            "[[snippet]]\nname = \"\"\ntext = \"x\"\n",
            "[[snippet]]\nname = \"   \"\ntext = \"x\"\n",
            "[[snippet]]\nname = \"x\"\ntext = \"\"\n",
            "[[snippet]]\nname = \"x\"\ntext = 3\n",
            "[[snippet]]\nname = \"x\"\ntext = \"y\"\nkeywords = \"not a list\"\n",
            "[[snippet]]\nname = \"x\"\ntext = \"y\"\ntypo = 1\n",
            "snippets = []\n",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn missing_file_is_empty_with_a_hint_and_malformed_reports() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dojo/snippets.toml");
        assert!(load(&path).unwrap().is_empty());

        let loaded = LoadedSnippets::from_path(&path);
        assert!(loaded.snippets.is_empty() && loaded.error.is_none());
        let hint = loaded.empty_hint().unwrap();
        assert!(
            hint.contains("No snippets yet") && hint.contains("snippets.toml"),
            "{hint}"
        );

        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "[[snippet]\n").unwrap();
        assert!(load(&path).is_err());
        let loaded = LoadedSnippets::from_path(&path);
        assert!(loaded.snippets.is_empty());
        assert!(loaded.error.is_some());
        assert!(loaded.empty_hint().unwrap().contains("could not be read"));

        fs::write(&path, "[[snippet]]\nname = \"a\"\ntext = \"b\"\n").unwrap();
        let loaded = LoadedSnippets::from_path(&path);
        assert_eq!(loaded.snippets.len(), 1);
        assert_eq!(loaded.empty_hint(), None);
    }

    #[test]
    fn unreadable_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load(dir.path()).is_err());
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_fuzzy() {
        let snippets = vec![
            snippet("Meeting notes template", &[], "a"),
            snippet("Email", &["mail"], "b"),
            snippet("Gmail signature", &[], "c"),
            snippet("Mailing address", &[], "d"),
            snippet("Mail", &[], "e"),
            snippet("Home", &["email"], "f"),
        ];
        // Exact name, exact keyword, name prefix, fuzzy name, then fuzzy keyword (Home via `email`).
        assert_eq!(
            names(&snippets, "mail"),
            [
                "Mail",
                "Email",
                "Mailing address",
                "Gmail signature",
                "Home"
            ]
        );
        // Smart case, as in the application search: a lowercase query ignores case, a query with
        // a capital matches case-sensitively.
        assert_eq!(names(&snippets, "Mail"), ["Mail", "Mailing address"]);
        assert!(names(&snippets, "MAIL").is_empty());
        // An exact keyword outranks a name prefix, and a name outranks the keyword.
        assert_eq!(names(&snippets, "email"), ["Email", "Home"]);
        assert_eq!(names(&snippets, "ho"), ["Home"]);
    }

    #[test]
    fn keyword_prefix_and_fuzzy_rank_below_name_matches() {
        let snippets = vec![
            snippet("Zeta", &["alpha"], "a"),
            snippet("Alphabet soup", &[], "b"),
            snippet("Gamma", &["alphanumeric"], "c"),
            snippet("Completely different", &["al-pha"], "d"),
            snippet("Alpha", &[], "e"),
        ];
        assert_eq!(
            names(&snippets, "alpha"),
            [
                "Alpha",
                "Zeta",
                "Alphabet soup",
                "Gamma",
                "Completely different"
            ]
        );
    }

    #[test]
    fn search_finds_by_keyword_and_handles_empty_and_unmatched_queries() {
        let snippets = vec![
            snippet("Sign-off", &["sig", "regards"], "a"),
            snippet("Address", &[], "b"),
        ];
        assert_eq!(names(&snippets, "regards"), ["Sign-off"]);
        assert_eq!(names(&snippets, "rgds"), ["Sign-off"]);
        assert_eq!(names(&snippets, ""), ["Sign-off", "Address"]);
        assert_eq!(names(&snippets, "   "), ["Sign-off", "Address"]);
        assert!(names(&snippets, "zzzz").is_empty());
        assert!(names(&[], "x").is_empty());
    }

    #[test]
    fn search_is_stable_in_file_order_and_matches_unicode() {
        let snippets = vec![
            snippet("Notes B", &[], "a"),
            snippet("Notes A", &[], "b"),
            snippet("Café order", &[], "c"),
        ];
        assert_eq!(names(&snippets, "notes"), ["Notes B", "Notes A"]);
        assert_eq!(names(&snippets, "cafe"), ["Café order"]);
        assert_eq!(names(&snippets, "café"), ["Café order"]);
    }

    #[test]
    fn search_is_limited() {
        let snippets = (0..200)
            .map(|n| snippet(&format!("snippet {n}"), &[], "x"))
            .collect::<Vec<_>>();
        assert_eq!(names(&snippets, "snippet").len(), RESULT_LIMIT);
        assert_eq!(names(&snippets, "").len(), RESULT_LIMIT);
    }

    #[test]
    fn every_placeholder_expands() {
        assert_eq!(expanded("{date}"), "2026-03-07");
        assert_eq!(expanded("{time}"), "09:05");
        assert_eq!(expanded("{datetime}"), "2026-03-07 09:05");
        assert_eq!(expanded("{clipboard}"), "copied text");
        assert_eq!(expanded("{uuid}"), "01010101-0101-4101-8101-010101010101");
        assert_eq!(
            expanded("On {date} at {time}: {clipboard}"),
            "On 2026-03-07 at 09:05: copied text"
        );
    }

    #[test]
    fn uuid_is_new_for_every_occurrence_and_well_formed() {
        let both = expanded("{uuid} {uuid}");
        let (first, second) = both.split_once(' ').unwrap();
        assert_ne!(first, second);

        let context = SystemContext::default();
        let value = expand_text("{uuid}", &context);
        let parsed = uuid::Uuid::parse_str(&value).unwrap();
        assert_eq!(parsed.get_version_num(), 4);
        assert_ne!(value, expand_text("{uuid}", &context));
    }

    #[test]
    fn dates_are_zero_padded() {
        let context = Fixed {
            now: DateTime {
                year: 987,
                month: 1,
                day: 2,
                hour: 0,
                minute: 0,
            },
            ..Fixed::new()
        };
        assert_eq!(expand_text("{datetime}", &context), "0987-01-02 00:00");
        let context = Fixed {
            now: DateTime {
                year: 2030,
                month: 12,
                day: 31,
                hour: 23,
                minute: 59,
            },
            ..Fixed::new()
        };
        assert_eq!(expand_text("{datetime}", &context), "2030-12-31 23:59");
    }

    #[test]
    fn doubled_braces_are_literal() {
        assert_eq!(expanded("{{date}}"), "{date}");
        assert_eq!(expanded("{{"), "{");
        assert_eq!(expanded("}}"), "}");
        assert_eq!(expanded("fn main() {{ }}"), "fn main() { }");
        assert_eq!(expanded("{{{date}}}"), "{2026-03-07}");
        assert_eq!(expanded("{{ {date} }}"), "{ 2026-03-07 }");
    }

    #[test]
    fn unknown_placeholders_and_stray_braces_stay_as_typed() {
        for text in [
            "{name}",
            "{DATE}",
            "{ date }",
            "{}",
            "{",
            "}",
            "{date",
            "date}",
            "a { b",
            "a } b",
            "{unclosed {name}",
            "{date:%Y}",
            "{clipboard2}",
            "{{}",
        ] {
            let expected = text.replace("{{", "{").replace("}}", "}");
            // Anything that is not a known placeholder comes out unchanged (apart from `{{`).
            assert_eq!(expanded(text), expected, "{text}");
        }
        assert_eq!(expanded("{unclosed {date}"), "{unclosed 2026-03-07");
        assert_eq!(expanded("{date}}"), "2026-03-07}");
        assert_eq!(expanded("{name} on {date}"), "{name} on 2026-03-07");
    }

    #[test]
    fn inserted_values_are_not_expanded_again() {
        let context = Fixed {
            clipboard: "{date} {{ }} {uuid}",
            ..Fixed::new()
        };
        assert_eq!(
            expand_text("[{clipboard}]", &context),
            "[{date} {{ }} {uuid}]"
        );
    }

    #[test]
    fn unicode_and_multiline_text_survive_expansion() {
        assert_eq!(
            expanded("Grüße 🦀 {date}\n\t日本語 — {time}\r\n"),
            "Grüße 🦀 2026-03-07\n\t日本語 — 09:05\r\n"
        );
        assert_eq!(expanded("日本{date}語"), "日本2026-03-07語");
        assert_eq!(expanded("é{"), "é{");
        assert_eq!(expanded(""), "");
    }

    #[test]
    fn expand_uses_the_snippet_text() {
        let snippet = snippet("Today", &[], "Today is {date}");
        assert_eq!(expand(&snippet, &Fixed::new()), "Today is 2026-03-07");
    }

    #[test]
    fn system_clock_gives_a_plausible_time() {
        let now = SystemContext::default().now();
        assert!(now.year >= 2024);
        assert!((1..=12).contains(&now.month) && (1..=31).contains(&now.day));
        assert!(now.hour < 24 && now.minute < 60);
        assert_eq!(now.date().len(), 10);
    }

    #[test]
    fn append_creates_the_file_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/dojo/snippets.toml");
        let tricky = snippet(
            "Tricky \"quoted\" 🦀",
            &["k1", "ключ"],
            "line one\nline \"two\" with \\ backslash, {{braces}} and {date}\n\ttabbed — ünï\n",
        );
        append(&path, &tricky).unwrap();
        assert_eq!(load(&path).unwrap(), [tricky.clone()]);

        let plain = snippet("Plain", &[], "just text");
        append(&path, &plain).unwrap();
        assert_eq!(load(&path).unwrap(), [tricky, plain]);
    }

    #[test]
    fn append_keeps_comments_and_hand_written_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        let original =
            "# my snippets\n[[snippet]]\nname = \"a\"\ntext = 'literal \\n'  # no trailing newline";
        fs::write(&path, original).unwrap();
        append(&path, &snippet("b", &["x"], "second")).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(original), "{text}");
        let snippets = load(&path).unwrap();
        assert_eq!(snippets.len(), 2);
        assert_eq!(snippets[0].text, "literal \\n");
        assert_eq!(snippets[1], snippet("b", &["x"], "second"));
    }

    #[test]
    fn append_refuses_to_overwrite_a_malformed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        fs::write(&path, "[[snippet]\nname = oops").unwrap();
        assert!(append(&path, &snippet("b", &[], "t")).is_err());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[[snippet]\nname = oops"
        );
    }

    #[test]
    fn append_and_save_all_reject_invalid_snippets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        assert!(append(&path, &snippet("", &[], "t")).is_err());
        assert!(append(&path, &snippet("n", &[], "")).is_err());
        assert!(save_all(&path, &[snippet("  ", &[], "t")]).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn save_all_replaces_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        append(&path, &snippet("old", &[], "old")).unwrap();
        let replacement = vec![
            snippet("one", &["1"], "first"),
            snippet("two", &[], "second"),
        ];
        save_all(&path, &replacement).unwrap();
        assert_eq!(load(&path).unwrap(), replacement);
        save_all(&path, &[]).unwrap();
        assert!(load(&path).unwrap().is_empty());
    }

    #[test]
    fn saving_leaves_no_temporary_files_and_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        for n in 0..5 {
            append(&path, &snippet(&format!("s{n}"), &[], "t")).unwrap();
        }
        let entries = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(entries, ["snippets.toml"]);
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(load(&path).unwrap().len(), 5);
    }

    #[test]
    fn saving_keeps_the_mode_of_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        append(&path, &snippet("a", &[], "b")).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("snippets.toml");
        append(&path, &snippet("keep", &[], "me")).unwrap();
        // Renaming a file over a directory fails after the temporary file was written.
        let blocked = dir.path().join("blocked.toml");
        fs::create_dir(&blocked).unwrap();
        assert!(write_atomic(&blocked, "x = 1\n").is_err());
        assert!(blocked.is_dir());
        let leftovers = fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 2, "snippets.toml and blocked.toml only");
        assert_eq!(load(&path).unwrap().len(), 1);
    }

    #[test]
    fn user_config_path_has_the_expected_shape() {
        if let Ok(path) = user_config_path() {
            assert!(path.ends_with("dojo/snippets.toml"));
        }
    }
}
