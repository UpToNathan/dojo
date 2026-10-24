//! 08 Bangs that open in Helium. See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! A bang is a short trigger (`yt`) for a site. `!yt cats` and `cats !yt` resolve to the finished
//! search URL, a bare `!yt` to the site's home page, and the launcher opens the result in Helium.
//! The tracked list is `data/launcher-bangs.toml`, embedded at build time. A user file at
//! `$XDG_CONFIG_HOME/dojo/bangs.toml` (fallback `~/.config/dojo/bangs.toml`) uses the same format,
//! adds bangs, replaces tracked ones by trigger, and may name the `default_engine` that the
//! launcher's "search the web" fallback uses.
//!
//! ## Query encoding
//!
//! The query is percent-encoded byte by byte (UTF-8), keeping only the RFC 3986 unreserved
//! characters `A-Z a-z 0-9 - . _ ~`. Everything else, including `&`, `#`, `+`, `/`, `=` and `%`,
//! becomes `%XX` with uppercase hex, so the query can never add a parameter, a path segment, a
//! fragment, or change the scheme or host. The one rule that depends on the template is the space:
//! when `{}` sits in the query string (after a `?`) a space becomes `+`, the form encoding every
//! search site expects; anywhere else (a path such as `open.spotify.com/search/{}`) it becomes
//! `%20`, because `+` is a literal plus in a path.
//!
//! ## Validation
//!
//! Templates are checked when loaded: scheme `https` (tracked list) or `https`/`http` (user
//! entries), a non-empty host without user-info, no whitespace, and exactly one `{}` which must
//! come after the host. Home URLs obey the same scheme and host rules and contain no `{}`.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

const TRACKED_BANGS: &str = include_str!("../../data/launcher-bangs.toml");
const DEFAULT_ENGINE: &str = "ddg";
const PLACEHOLDER: &str = "{}";

/// One site the launcher can search or open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bang {
    /// Lowercase, no whitespace, no leading `!`.
    pub trigger: String,
    pub name: String,
    /// Search URL with a single `{}` where the query goes.
    pub search_template: String,
    /// Where a bare bang goes.
    pub home_url: String,
    /// The site may ignore the query or block automated checks (the AI chat sites).
    pub best_effort: bool,
    encoding: Encoding,
}

/// How a space is written, decided by where `{}` sits in the template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    /// `{}` is in the query string: space becomes `+`.
    Query,
    /// `{}` is in the path: space becomes `%20`.
    Path,
}

/// A resolved bang: what to open and how to label the row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BangMatch {
    pub bang: Bang,
    pub url: String,
    /// Row title, such as `Search YouTube for “cats”` or `Open YouTube`.
    pub title: String,
    /// The search text, or `None` for a bare bang that opens the home page.
    pub query: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BangSet {
    bangs: Vec<Bang>,
    default_engine: String,
}

/// A bang set plus the reason the user file was not applied, if it was not.
#[derive(Clone, Debug)]
pub struct LoadedBangs {
    pub set: BangSet,
    /// Present when the user file existed but could not be used; `set` is then the tracked list.
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    default_engine: Option<String>,
    #[serde(default)]
    bang: Vec<RawBang>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBang {
    trigger: String,
    name: Option<String>,
    url: String,
    home: Option<String>,
    #[serde(default)]
    best_effort: bool,
}

impl Bang {
    fn from_raw(raw: RawBang, allow_http: bool) -> Result<Self> {
        let trigger = raw.trigger.trim().to_lowercase();
        if trigger.is_empty() {
            bail!("a bang has an empty trigger");
        }
        if trigger.starts_with('!') {
            bail!("bang trigger {trigger:?} must not start with `!`");
        }
        if trigger.chars().any(char::is_whitespace) {
            bail!("bang trigger {trigger:?} must not contain whitespace");
        }
        let context = |what: &str| format!("bang `{trigger}`: {what}");

        let (origin, rest) = split_origin(&raw.url, allow_http)
            .with_context(|| context(&format!("invalid url {:?}", raw.url)))?;
        if raw.url.matches(PLACEHOLDER).count() != 1 || origin.contains(PLACEHOLDER) {
            bail!(
                "{}",
                context("the url needs exactly one `{}` after the host")
            );
        }
        let before = &rest[..rest.find(PLACEHOLDER).expect("one placeholder is present")];
        let encoding = if before.contains('?') {
            Encoding::Query
        } else {
            Encoding::Path
        };

        let home_url = match raw.home {
            Some(home) => {
                if home.contains(PLACEHOLDER) {
                    bail!("{}", context("the home url must not contain `{}`"));
                }
                split_origin(&home, allow_http)
                    .with_context(|| context(&format!("invalid home url {home:?}")))?;
                home
            }
            None => format!("{origin}/"),
        };
        let name = raw
            .name
            .map(|name| name.trim().to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| trigger.clone());
        Ok(Self {
            trigger,
            name,
            search_template: raw.url,
            home_url,
            best_effort: raw.best_effort,
            encoding,
        })
    }

    /// The template with `query` encoded into it. The query may be empty.
    pub fn search_url(&self, query: &str) -> String {
        let encoded = encode_query(query, self.encoding);
        self.search_template.replacen(PLACEHOLDER, &encoded, 1)
    }

    fn match_for(&self, query: Option<&str>) -> BangMatch {
        match query {
            Some(query) => BangMatch {
                bang: self.clone(),
                url: self.search_url(query),
                title: format!("Search {} for “{query}”", self.name),
                query: Some(query.to_owned()),
            },
            None => BangMatch {
                bang: self.clone(),
                url: self.home_url.clone(),
                title: format!("Open {}", self.name),
                query: None,
            },
        }
    }
}

impl BangSet {
    /// The list checked into the repository.
    pub fn tracked() -> Self {
        Self::parse(TRACKED_BANGS, false).expect("data/launcher-bangs.toml is valid (see tests)")
    }

    /// The tracked list with the user file merged in. A missing file is the tracked list; a file
    /// that cannot be read or does not validate is an error and nothing from it is applied.
    pub fn load_user(path: &Path) -> Result<Self> {
        Self::tracked().merge_user_file(path)
    }

    /// The tracked list, with the user file at the default location merged in. A broken user file
    /// never breaks the launcher: the tracked list is returned and the reason is in `error`.
    pub fn for_user() -> LoadedBangs {
        match user_config_path() {
            Ok(path) => Self::load_or_tracked(&path),
            Err(error) => LoadedBangs {
                set: Self::tracked(),
                error: Some(format!("{error:#}")),
            },
        }
    }

    /// Like [`BangSet::load_user`], but falls back to the tracked list and reports the error.
    pub fn load_or_tracked(path: &Path) -> LoadedBangs {
        match Self::load_user(path) {
            Ok(set) => LoadedBangs { set, error: None },
            Err(error) => LoadedBangs {
                set: Self::tracked(),
                error: Some(format!("{error:#}")),
            },
        }
    }

    /// Merge the user file at `path` over this set. See [`BangSet::load_user`].
    pub fn merge_user_file(&self, path: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(text) => self
                .merge_user_toml(&text)
                .with_context(|| format!("in {}", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(self.clone()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Merge user TOML over this set: entries replace a bang with the same trigger in place and
    /// new triggers are appended. `default_engine` must name a bang in the merged set.
    pub fn merge_user_toml(&self, text: &str) -> Result<Self> {
        let file: RawFile = toml::from_str(text).context("parsing bangs")?;
        let mut merged = self.clone();
        let mut seen = HashSet::new();
        for raw in file.bang {
            let bang = Bang::from_raw(raw, true)?;
            if !seen.insert(bang.trigger.clone()) {
                bail!("bang `{}` is defined twice", bang.trigger);
            }
            match merged.position(&bang.trigger) {
                Some(index) => merged.bangs[index] = bang,
                None => merged.bangs.push(bang),
            }
        }
        if let Some(engine) = file.default_engine {
            let engine = engine.trim().trim_start_matches('!').to_lowercase();
            if merged.position(&engine).is_none() {
                bail!("default_engine `{engine}` is not a known bang");
            }
            merged.default_engine = engine;
        }
        Ok(merged)
    }

    fn parse(text: &str, allow_http: bool) -> Result<Self> {
        let file: RawFile = toml::from_str(text).context("parsing bangs")?;
        let mut bangs: Vec<Bang> = Vec::new();
        for raw in file.bang {
            let bang = Bang::from_raw(raw, allow_http)?;
            if bangs.iter().any(|known| known.trigger == bang.trigger) {
                bail!("bang `{}` is defined twice", bang.trigger);
            }
            bangs.push(bang);
        }
        let default_engine = file.default_engine.unwrap_or_else(|| DEFAULT_ENGINE.into());
        let set = Self {
            bangs,
            default_engine,
        };
        if set.position(&set.default_engine).is_none() {
            bail!(
                "default_engine `{}` is not a known bang",
                set.default_engine
            );
        }
        Ok(set)
    }

    fn position(&self, trigger: &str) -> Option<usize> {
        self.bangs.iter().position(|bang| bang.trigger == trigger)
    }

    pub fn bangs(&self) -> &[Bang] {
        &self.bangs
    }

    /// Look a bang up by trigger, ignoring case. The trigger has no leading `!`.
    pub fn get(&self, trigger: &str) -> Option<&Bang> {
        let trigger = trigger.to_lowercase();
        self.position(&trigger).map(|index| &self.bangs[index])
    }

    pub fn default_engine(&self) -> &Bang {
        self.get(&self.default_engine)
            .expect("the default engine is validated when the set is built")
    }

    /// Resolve `!yt cats` (leading), `cats !yt` (trailing, preceded by whitespace) or `!yt`
    /// (bare, the home page). Only the first or last whitespace-separated token can be a bang;
    /// the first is tried before the last. Unknown triggers and ordinary text give `None`, so
    /// `hello!` and `a!b` are not bangs.
    pub fn resolve(&self, query: &str) -> Option<BangMatch> {
        let query = query.trim();
        if let Some(bang) = query
            .split_whitespace()
            .next()
            .and_then(|t| self.bang_token(t))
        {
            let rest = match query.split_once(char::is_whitespace) {
                Some((_, rest)) => rest.trim(),
                None => "",
            };
            return Some(bang.match_for((!rest.is_empty()).then_some(rest)));
        }
        let (rest, last) = query.rsplit_once(char::is_whitespace)?;
        let bang = self.bang_token(last)?;
        Some(bang.match_for(Some(rest.trim())))
    }

    /// The "search the web" fallback: the whole query goes to the default engine, with no bang
    /// parsing. An empty query gives `None`.
    pub fn web_search(&self, query: &str) -> Option<BangMatch> {
        let query = query.trim();
        if query.is_empty() {
            return None;
        }
        Some(self.default_engine().match_for(Some(query)))
    }

    fn bang_token(&self, token: &str) -> Option<&Bang> {
        self.get(token.strip_prefix('!')?)
    }
}

/// `$XDG_CONFIG_HOME/dojo/bangs.toml`, falling back to `~/.config/dojo/bangs.toml`.
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
    Ok(config_home.join("dojo/bangs.toml"))
}

/// Split `url` into its origin (`https://host[:port]`) and the rest (path, query, fragment).
/// Only `https`, or `http` when `allow_http`, are accepted; the host must be non-empty and the
/// URL free of whitespace, control characters and user-info.
fn split_origin(url: &str, allow_http: bool) -> Option<(&str, &str)> {
    if url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let lower = url.to_ascii_lowercase();
    let scheme_len = if lower.starts_with("https://") {
        "https://".len()
    } else if allow_http && lower.starts_with("http://") {
        "http://".len()
    } else {
        return None;
    };
    let after_scheme = &url[scheme_len..];
    let authority_len = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    let authority = &after_scheme[..authority_len];
    if authority.is_empty() || authority.contains('@') || authority.starts_with(':') {
        return None;
    }
    let split = scheme_len + authority_len;
    Some((&url[..split], &url[split..]))
}

fn encode_query(query: &str, encoding: Encoding) -> String {
    let mut encoded = String::with_capacity(query.len());
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            b' ' if encoding == Encoding::Query => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(set: &BangSet, query: &str) -> String {
        set.resolve(query)
            .unwrap_or_else(|| panic!("{query:?} should resolve"))
            .url
    }

    #[test]
    fn leading_bang_resolves() {
        let set = BangSet::tracked();
        let found = set.resolve("!yt cats").unwrap();
        assert_eq!(
            found.url,
            "https://www.youtube.com/results?search_query=cats"
        );
        assert_eq!(found.bang.trigger, "yt");
        assert_eq!(found.query.as_deref(), Some("cats"));
        assert_eq!(found.title, "Search YouTube for “cats”");
    }

    #[test]
    fn trailing_bang_resolves() {
        let set = BangSet::tracked();
        assert_eq!(
            url(&set, "funny cats !yt"),
            "https://www.youtube.com/results?search_query=funny+cats"
        );
        assert_eq!(
            url(&set, "  cats   !YT  "),
            "https://www.youtube.com/results?search_query=cats"
        );
    }

    #[test]
    fn trailing_bang_needs_a_space_before_it() {
        let set = BangSet::tracked();
        assert_eq!(set.resolve("cats!yt"), None);
        assert_eq!(set.resolve("cats !ytx"), None);
    }

    #[test]
    fn bare_bang_opens_the_home_page() {
        let set = BangSet::tracked();
        let found = set.resolve("!yt").unwrap();
        assert_eq!(found.url, "https://www.youtube.com/");
        assert_eq!(found.query, None);
        assert_eq!(found.title, "Open YouTube");
        assert_eq!(url(&set, "  !gh  "), "https://github.com/");
        // An explicit home wins over the host default.
        assert_eq!(url(&set, "!std"), "https://doc.rust-lang.org/std/");
        assert_eq!(url(&set, "!maps"), "https://www.google.com/maps");
    }

    #[test]
    fn unknown_bangs_and_ordinary_text_are_not_bangs() {
        let set = BangSet::tracked();
        assert_eq!(set.resolve("!nope cats"), None);
        assert_eq!(set.resolve("cats !nope"), None);
        assert_eq!(set.resolve("!nope"), None);
        assert_eq!(set.resolve("!"), None);
        assert_eq!(set.resolve("! yt"), None);
        assert_eq!(set.resolve("hello!"), None);
        assert_eq!(set.resolve("yt cats"), None);
        assert_eq!(set.resolve("yt"), None);
        assert_eq!(set.resolve("wow! yt!"), None);
        assert_eq!(set.resolve(""), None);
        assert_eq!(set.resolve("   "), None);
    }

    #[test]
    fn only_the_first_or_last_token_counts() {
        let set = BangSet::tracked();
        assert_eq!(set.resolve("cats !yt dogs"), None);
        // First wins when both are bangs; the other stays in the query.
        let found = set.resolve("!yt cats !g").unwrap();
        assert_eq!(found.bang.trigger, "yt");
        assert_eq!(found.query.as_deref(), Some("cats !g"));
        // An unknown leading token does not hide a known trailing bang.
        let found = set.resolve("!nope cats !g").unwrap();
        assert_eq!(found.bang.trigger, "g");
        assert_eq!(found.query.as_deref(), Some("!nope cats"));
    }

    #[test]
    fn triggers_are_case_insensitive() {
        let set = BangSet::tracked();
        assert_eq!(url(&set, "!YT cats"), url(&set, "!yt cats"));
        assert_eq!(url(&set, "cats !Yt"), url(&set, "!yt cats"));
        assert!(set.get("GH").is_some());
    }

    #[test]
    fn query_text_is_kept_as_typed_apart_from_edge_whitespace() {
        let set = BangSet::tracked();
        let found = set.resolve("!g   a  b ").unwrap();
        assert_eq!(found.query.as_deref(), Some("a  b"));
        assert_eq!(found.url, "https://www.google.com/search?q=a++b");
    }

    #[test]
    fn tricky_input_is_percent_encoded_in_query_strings() {
        let set = BangSet::tracked();
        assert_eq!(
            url(&set, "!g a&b=c#d+e"),
            "https://www.google.com/search?q=a%26b%3Dc%23d%2Be"
        );
        assert_eq!(
            url(&set, "!g c++ 100%"),
            "https://www.google.com/search?q=c%2B%2B+100%25"
        );
        assert_eq!(
            url(&set, "!g crème brûlée 日本"),
            "https://www.google.com/search?q=cr%C3%A8me+br%C3%BBl%C3%A9e+%E6%97%A5%E6%9C%AC"
        );
        assert_eq!(
            url(&set, "!g 🦀"),
            "https://www.google.com/search?q=%F0%9F%A6%80"
        );
        assert_eq!(
            url(&set, "!g a/b?c\"d'e<f>"),
            "https://www.google.com/search?q=a%2Fb%3Fc%22d%27e%3Cf%3E"
        );
        // Unreserved punctuation stays readable.
        assert_eq!(
            url(&set, "!g a-b_c.d~e"),
            "https://www.google.com/search?q=a-b_c.d~e"
        );
        // A tab or newline inside the query cannot break out of the URL.
        assert_eq!(
            url(&set, "!g a\tb"),
            "https://www.google.com/search?q=a%09b"
        );
    }

    #[test]
    fn path_templates_use_percent_twenty_for_spaces() {
        let set = BangSet::tracked();
        assert_eq!(
            url(&set, "!sp daft punk"),
            "https://open.spotify.com/search/daft%20punk"
        );
        assert_eq!(
            url(&set, "!sp a/b+c"),
            "https://open.spotify.com/search/a%2Fb%2Bc"
        );
        assert_eq!(
            url(&set, "!maps coffee near me"),
            "https://www.google.com/maps/search/coffee%20near%20me"
        );
    }

    #[test]
    fn the_query_cannot_change_scheme_or_host() {
        let set = BangSet::tracked();
        for bang in set.bangs() {
            for hostile in [
                "https://evil.example/x",
                "//evil.example",
                "@evil.example",
                "\\\\evil.example",
                "../../etc/passwd",
                "x#@evil.example",
                "{}",
            ] {
                let built = bang.search_url(hostile);
                let origin = split_origin(&bang.search_template, false).unwrap().0;
                assert!(built.starts_with(origin), "{built}");
                let after = &built[origin.len()..];
                assert!(!after.contains("://") && !after.contains("evil.example/"));
                assert_eq!(
                    split_origin(&built, false).unwrap().0,
                    origin,
                    "origin changed for {}",
                    bang.trigger
                );
                // Only template characters remain unescaped: no extra `#`, `@` or `\`.
                let tail = &built[bang.search_template.find("{}").unwrap()..];
                assert!(!tail.contains(['#', '@', '\\', ' ']), "{built}");
            }
        }
    }

    #[test]
    fn web_search_uses_the_default_engine() {
        let set = BangSet::tracked();
        assert_eq!(set.default_engine().trigger, "ddg");
        let found = set.web_search("rust lifetimes").unwrap();
        assert_eq!(found.url, "https://duckduckgo.com/?q=rust+lifetimes");
        assert_eq!(found.title, "Search DuckDuckGo for “rust lifetimes”");
        // The fallback does not parse bangs: the text is searched as typed.
        assert_eq!(
            set.web_search("!yt cats").unwrap().url,
            "https://duckduckgo.com/?q=%21yt+cats"
        );
        assert_eq!(set.web_search("  "), None);
    }

    #[test]
    fn user_file_adds_overrides_and_picks_the_default_engine() {
        let set = BangSet::tracked()
            .merge_user_toml(
                r#"
default_engine = "!Kagi"

[[bang]]
trigger = "G"
name = "Google (UK)"
url = "https://www.google.co.uk/search?q={}"

[[bang]]
trigger = "lab"
name = "Lab wiki"
url = "http://wiki.lab.internal:8080/search/{}"
home = "http://wiki.lab.internal:8080/"
"#,
            )
            .unwrap();
        assert_eq!(set.default_engine().trigger, "kagi");
        assert_eq!(url(&set, "!g tea"), "https://www.google.co.uk/search?q=tea");
        assert_eq!(set.get("g").unwrap().name, "Google (UK)");
        assert_eq!(
            url(&set, "!lab on call"),
            "http://wiki.lab.internal:8080/search/on%20call"
        );
        assert_eq!(url(&set, "!lab"), "http://wiki.lab.internal:8080/");
        assert_eq!(set.bangs().len(), BangSet::tracked().bangs().len() + 1);
        // The override replaced the tracked entry in place and kept the order.
        assert_eq!(set.bangs()[0].trigger, "g");
        // Untouched tracked bangs still work.
        assert_eq!(url(&set, "!gh tokio"), "https://github.com/search?q=tokio");
        assert_eq!(set.web_search("x").unwrap().bang.trigger, "kagi");
    }

    #[test]
    fn user_name_defaults_to_the_trigger_and_home_to_the_host() {
        let set = BangSet::tracked()
            .merge_user_toml(
                "[[bang]]\ntrigger = \"wk\"\nurl = \"https://wiki.example/find?q={}\"\n",
            )
            .unwrap();
        let bang = set.get("wk").unwrap();
        assert_eq!(bang.name, "wk");
        assert_eq!(bang.home_url, "https://wiki.example/");
        assert!(!bang.best_effort);
    }

    #[test]
    fn user_file_is_read_from_disk_and_a_missing_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("bangs.toml");
        assert_eq!(BangSet::load_user(&missing).unwrap(), BangSet::tracked());
        let loaded = BangSet::load_or_tracked(&missing);
        assert!(loaded.error.is_none());
        assert_eq!(loaded.set, BangSet::tracked());

        fs::write(
            &missing,
            "[[bang]]\ntrigger = \"wk\"\nurl = \"https://wiki.example/find?q={}\"\n",
        )
        .unwrap();
        let loaded = BangSet::load_or_tracked(&missing);
        assert!(loaded.error.is_none());
        assert!(loaded.set.get("wk").is_some());
    }

    #[test]
    fn malformed_user_files_report_an_error_and_keep_the_tracked_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bangs.toml");
        for bad in [
            "this is not toml [[[",
            "[[bang]]\ntrigger = \"x\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example/{}/{}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example/\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://{}.example/\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example?q={}\"\nhome = \"https://a.example/{}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"ftp://a.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"javascript:alert({})\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://user@a.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https:///path?q={}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example/?q={}\"\nhome = \"file:///etc\"\n",
            "[[bang]]\ntrigger = \"\"\nurl = \"https://a.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"!x\"\nurl = \"https://a.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"a b\"\nurl = \"https://a.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example/?q={}\"\n[[bang]]\ntrigger = \"X\"\nurl = \"https://b.example/?q={}\"\n",
            "[[bang]]\ntrigger = \"x\"\nurl = \"https://a.example/?q={}\"\nbogus = 1\n",
            "default_engine = \"nonexistent\"\n",
        ] {
            fs::write(&path, bad).unwrap();
            let error = BangSet::load_user(&path).expect_err(bad);
            assert!(!format!("{error:#}").is_empty());
            let loaded = BangSet::load_or_tracked(&path);
            assert_eq!(loaded.set, BangSet::tracked(), "{bad}");
            assert!(loaded.error.is_some(), "{bad}");
        }
    }

    #[test]
    fn a_bad_entry_applies_nothing_from_the_file() {
        let error = BangSet::tracked()
            .merge_user_toml(
                "[[bang]]\ntrigger = \"ok\"\nurl = \"https://a.example/?q={}\"\n\
                 [[bang]]\ntrigger = \"bad\"\nurl = \"nope\"\n",
            )
            .unwrap_err();
        assert!(format!("{error:#}").contains("bad"), "{error:#}");
    }

    #[test]
    fn unreadable_user_path_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be.
        let loaded = BangSet::load_or_tracked(dir.path());
        assert!(loaded.error.is_some());
        assert_eq!(loaded.set, BangSet::tracked());
    }

    #[test]
    fn tracked_list_is_valid() {
        let text: RawFile = toml::from_str(TRACKED_BANGS).unwrap();
        // Parsing the raw text again checks duplicates, schemes and placeholders.
        let set = BangSet::parse(TRACKED_BANGS, false).unwrap();
        assert_eq!(set.bangs().len(), text.bang.len());
        assert!(set.bangs().len() >= 40);

        let mut triggers = HashSet::new();
        for bang in set.bangs() {
            assert!(
                triggers.insert(bang.trigger.clone()),
                "duplicate {}",
                bang.trigger
            );
            assert_eq!(bang.trigger, bang.trigger.to_lowercase());
            assert!(!bang.trigger.contains(char::is_whitespace) && !bang.trigger.starts_with('!'));
            assert!(
                bang.search_template.starts_with("https://"),
                "{}",
                bang.trigger
            );
            assert!(bang.home_url.starts_with("https://"), "{}", bang.trigger);
            assert_eq!(
                bang.search_template.matches("{}").count(),
                1,
                "{}",
                bang.trigger
            );
            assert!(!bang.home_url.contains("{}"));
            assert!(!bang.name.is_empty());
            // The home page is on the same site as the search.
            let (search_origin, _) = split_origin(&bang.search_template, false).unwrap();
            let (home_origin, _) = split_origin(&bang.home_url, false).unwrap();
            let registrable = |origin: &str| {
                origin
                    .trim_start_matches("https://")
                    .rsplit('.')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(".")
            };
            assert_eq!(
                registrable(search_origin),
                registrable(home_origin),
                "{}",
                bang.trigger
            );
        }
        for trigger in ["g", "ddg", "yt", "gh", "gpt", "claude", "pplx"] {
            assert!(set.get(trigger).is_some(), "{trigger}");
        }
        for trigger in ["gpt", "claude", "pplx"] {
            assert!(set.get(trigger).unwrap().best_effort);
        }
        assert!(!set.get("g").unwrap().best_effort);
    }

    #[test]
    fn tracked_default_engine_is_ddg() {
        assert_eq!(BangSet::tracked().default_engine().trigger, "ddg");
    }

    #[test]
    fn user_config_path_follows_xdg() {
        // Reads the process environment, which other tests do not set; just check the shape.
        if let Ok(path) = user_config_path() {
            assert!(path.ends_with("dojo/bangs.toml"));
        }
    }
}
