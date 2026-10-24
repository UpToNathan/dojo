use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process::Command;

use anyhow::Context as _;
use freedesktop_desktop_entry::DesktopEntry;
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};

use crate::launch_history::{LaunchHistory, TermMatch};

pub const RESULT_LIMIT: usize = 64;

pub struct Application {
    pub id: String,
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub icon: Option<String>,
    pub launch_command: Option<String>,
    pub startup_wm_class: Option<String>,
    pub last_used: Option<String>,
    searchable_name: Utf32String,
    searchable: Utf32String,
}

impl Application {
    /// An entry with no desktop file behind it, for tests and synthetic catalogues.
    pub fn new(id: &str, name: &str, description: &str) -> Self {
        Self {
            id: id.to_owned(),
            name: name.to_owned(),
            description: description.to_owned(),
            path: PathBuf::new(),
            icon: None,
            launch_command: None,
            startup_wm_class: None,
            last_used: None,
            searchable_name: Utf32String::from(name),
            searchable: Utf32String::from(format!("{name} {description}").as_str()),
        }
    }
}

/// One search result with what ordering needs. Lower tiers are better: 0 exact name or learned
/// term, 1 name prefix or learned prefix, 2 fuzzy name, 3 fuzzy match on the other metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scored {
    pub index: usize,
    pub tier: u8,
    pub score: u32,
    pub frecency: f64,
}

impl Scored {
    /// Best first: tier, then how much it has been launched, then the fuzzy score, then position.
    pub fn compare(&self, other: &Self) -> std::cmp::Ordering {
        self.tier
            .cmp(&other.tier)
            .then_with(|| other.frecency.total_cmp(&self.frecency))
            .then_with(|| other.score.cmp(&self.score))
            .then_with(|| self.index.cmp(&other.index))
    }
}

/// The patterns a query compiles to, shared by applications and launcher commands so both rank
/// by the same tiers.
pub struct QueryPatterns {
    fuzzy: Pattern,
    exact: Atom,
    prefix: Atom,
}

impl QueryPatterns {
    pub fn new(query: &str) -> Self {
        let atom = |kind| {
            Atom::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                kind,
                false,
            )
        };
        Self {
            fuzzy: Pattern::new(
                query,
                CaseMatching::Smart,
                Normalization::Smart,
                AtomKind::Fuzzy,
            ),
            exact: atom(AtomKind::Exact),
            prefix: atom(AtomKind::Prefix),
        }
    }

    /// Tier and score of a name plus its other searchable text, or `None` when neither matches.
    pub fn score(
        &self,
        name: &Utf32String,
        metadata: &Utf32String,
        matcher: &mut Matcher,
    ) -> Option<(u8, u32)> {
        if let Some(score) = self.fuzzy.score(name.slice(..), matcher) {
            let tier = if self.exact.score(name.slice(..), matcher).is_some() {
                0
            } else if self.prefix.score(name.slice(..), matcher).is_some() {
                1
            } else {
                2
            };
            Some((tier, score))
        } else {
            self.fuzzy
                .score(metadata.slice(..), matcher)
                .map(|score| (3, score))
        }
    }
}

pub struct Catalogue {
    pub applications: Vec<Application>,
}

impl Catalogue {
    pub fn discover() -> anyhow::Result<Self> {
        let roots = application_roots()?;
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .map(str::to_owned)
            .collect::<Vec<_>>();
        Self::from_roots(&roots, &locales, &desktops)
    }

    pub fn from_roots(
        roots: &[PathBuf],
        locales: &[String],
        desktops: &[String],
    ) -> anyhow::Result<Self> {
        let mut applications = Vec::new();
        let mut seen = HashSet::new();
        for root in roots {
            let mut files = Vec::new();
            collect_desktop_files(root, &mut files, &mut HashSet::new())?;
            files.sort();
            for path in files {
                let relative = path
                    .strip_prefix(root)
                    .context("desktop entry escaped its root")?;
                let id = relative.to_string_lossy().replace('/', "-");
                if !seen.insert(id.clone()) {
                    continue;
                }
                let entry = match DesktopEntry::from_path(&path, Some(locales)) {
                    Ok(entry) => entry,
                    Err(error) => {
                        eprintln!("Dojo skipped desktop entry {}: {error}", path.display());
                        continue;
                    }
                };
                if entry.type_() != Some("Application")
                    || entry.hidden()
                    || entry.no_display()
                    || (!entry.dbus_activatable() && entry.exec().is_none())
                    || entry
                        .try_exec()
                        .is_some_and(|program| !is_executable(program))
                    || !visible_on_desktop(&entry, desktops)
                {
                    continue;
                }
                let Some(name) = entry.name(locales).filter(|name| !name.is_empty()) else {
                    continue;
                };
                let name = name.into_owned();
                let description = entry
                    .generic_name(locales)
                    .map_or_else(String::new, |s| s.into_owned());
                let keywords = entry.keywords(locales).unwrap_or_default();
                let searchable = Utf32String::from(
                    format!(
                        "{name} {description} {} {id}",
                        keywords
                            .iter()
                            .map(|s| s.as_ref())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                    .as_str(),
                );
                applications.push(Application {
                    id,
                    searchable_name: Utf32String::from(name.as_str()),
                    name,
                    description,
                    last_used: fs::metadata(&path)
                        .ok()
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .and_then(|d| i64::try_from(d.as_secs()).ok())
                        .and_then(dojo_cua_core::unix_secs_to_rfc3339),
                    path,
                    icon: entry.icon().map(str::to_owned),
                    launch_command: entry.exec().map(strip_exec_field_codes),
                    startup_wm_class: entry.startup_wm_class().map(str::to_owned),
                    searchable,
                });
            }
        }
        applications.sort_by_cached_key(|app| app.name.to_lowercase());
        Ok(Self { applications })
    }

    pub fn search(&self, query: &str, matcher: &mut Matcher) -> Vec<usize> {
        self.search_ranked(query, matcher, &LaunchHistory::default(), 0)
    }

    /// Results for `query`, best first. Matches fall into tiers: an exact name or learned search
    /// term, then a name or learned-term prefix, then a fuzzy name match, then a fuzzy match on the
    /// rest of the metadata. Within a tier, applications launched more recently and more often
    /// come first, then the better fuzzy score. With no history this is the plain fuzzy order.
    /// An empty query lists the applications in use first.
    pub fn search_ranked(
        &self,
        query: &str,
        matcher: &mut Matcher,
        history: &LaunchHistory,
        now_ms: u64,
    ) -> Vec<usize> {
        if query.is_empty() {
            let mut used = self
                .applications
                .iter()
                .enumerate()
                .filter_map(|(index, app)| {
                    let frecency = history.frecency(&app.id, now_ms);
                    (frecency > 1.0).then_some((index, frecency))
                })
                .collect::<Vec<_>>();
            used.sort_by(|(left, left_weight), (right, right_weight)| {
                right_weight
                    .total_cmp(left_weight)
                    .then_with(|| left.cmp(right))
            });
            let mut order = used.iter().map(|(index, _)| *index).collect::<Vec<_>>();
            let used = order.iter().copied().collect::<HashSet<_>>();
            order.extend((0..self.applications.len()).filter(|index| !used.contains(index)));
            order.truncate(RESULT_LIMIT);
            return order;
        }
        let mut scored = self.scored(query, matcher, history, now_ms);
        scored.sort_unstable_by(Scored::compare);
        scored.truncate(RESULT_LIMIT);
        scored.into_iter().map(|scored| scored.index).collect()
    }

    /// Every application that matches a non-empty `query`, unordered, with what ranking needs.
    pub fn scored(
        &self,
        query: &str,
        matcher: &mut Matcher,
        history: &LaunchHistory,
        now_ms: u64,
    ) -> Vec<Scored> {
        let patterns = QueryPatterns::new(query);
        self.applications
            .iter()
            .enumerate()
            .filter_map(|(index, app)| {
                let matched = patterns.score(&app.searchable_name, &app.searchable, matcher);
                // A learned search term can promote a match or find an application the text
                // alone would not.
                let learned = match history.term_match(&app.id, query, now_ms) {
                    TermMatch::Exact => Some(0),
                    TermMatch::Prefix => Some(1),
                    TermMatch::None => None,
                };
                let (tier, score) = match (matched, learned) {
                    (Some((tier, score)), Some(learned)) => (tier.min(learned), score),
                    (Some(found), None) => found,
                    (None, Some(learned)) => (learned, 0),
                    (None, None) => return None,
                };
                Some(Scored {
                    index,
                    tier,
                    score,
                    frecency: history.frecency(&app.id, now_ms),
                })
            })
            .collect()
    }
}

// CUA's pinned Linux metadata conversion. This string is never executed here.
fn strip_exec_field_codes(exec: &str) -> String {
    let mut out = String::with_capacity(exec.len());
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            chars.next();
            continue;
        }
        out.push(c);
    }
    out.trim().to_owned()
}

pub fn launch(application: &Application) -> anyhow::Result<()> {
    crate::application_launch::launch(&application.path, &application.name)
}

#[cfg(test)]
fn desktop_session_environment() -> anyhow::Result<Vec<(String, String)>> {
    let connection = zbus::blocking::Connection::session()
        .context("connecting to the desktop session bus for application launch")?;
    let manager = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )?;
    let environment: Vec<String> = manager
        .get_property("Environment")
        .context("reading the user session manager's application environment")?;
    Ok(environment
        .into_iter()
        .filter_map(|assignment| {
            assignment
                .split_once('=')
                .map(|(key, value)| (key.to_owned(), value.to_owned()))
        })
        .collect())
}

fn application_roots() -> anyhow::Result<Vec<PathBuf>> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .context("neither XDG_DATA_HOME nor HOME is set")?;
    let mut roots = vec![home.join("applications")];
    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    roots.extend(
        std::env::split_paths(&data_dirs)
            .filter(|p| p.is_absolute())
            .map(|p| p.join("applications")),
    );
    Ok(roots)
}

fn collect_desktop_files(
    root: &Path,
    output: &mut Vec<PathBuf>,
    visited: &mut HashSet<PathBuf>,
) -> anyhow::Result<()> {
    let canonical = match fs::canonicalize(root) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", root.display())),
    };
    if !visited.insert(canonical) {
        return Ok(());
    }
    for entry in fs::read_dir(root).with_context(|| format!("scanning {}", root.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            collect_desktop_files(&path, output, visited)?;
        } else if path.extension().is_some_and(|ext| ext == "desktop") {
            output.push(path);
        }
    }
    Ok(())
}

fn is_executable(program: &str) -> bool {
    let check = |path: &Path| {
        fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return check(Path::new(program));
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| check(&dir.join(program))))
}

fn visible_on_desktop(entry: &DesktopEntry, desktops: &[String]) -> bool {
    if let Some(only) = entry.only_show_in() {
        if !only
            .iter()
            .any(|name| desktops.iter().any(|desktop| desktop == name))
        {
            return false;
        }
    }
    !entry.not_show_in().is_some_and(|excluded| {
        excluded
            .iter()
            .any(|name| desktops.iter().any(|desktop| desktop == name))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn application(name: &str, metadata: &str) -> Application {
        Application {
            id: format!("{name}.desktop"),
            name: name.into(),
            description: metadata.into(),
            path: PathBuf::new(),
            icon: None,
            launch_command: None,
            startup_wm_class: None,
            last_used: None,
            searchable_name: Utf32String::from(name),
            searchable: Utf32String::from(format!("{name} {metadata}").as_str()),
        }
    }

    #[test]
    fn name_prefixes_outrank_word_and_metadata_matches() {
        let catalogue = Catalogue {
            applications: vec![
                application("Avahi Zeroconf Browser", "Zeroconf discovery"),
                application("Zen Browser", "Web Browser"),
                application("Yazi File Manager", "File manager"),
                application("Other App", "Zen Browser"),
            ],
        };
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        assert_eq!(catalogue.search("z", &mut matcher).first(), Some(&1));
        assert_eq!(catalogue.search("zen", &mut matcher).first(), Some(&1));
        assert_eq!(
            catalogue.search("Zen Browser", &mut matcher).first(),
            Some(&1)
        );
    }

    fn used_catalogue() -> Catalogue {
        Catalogue {
            applications: vec![
                application("Files", "File manager"),
                application("Firefox", "Web Browser"),
                application("Fonts", "Font viewer"),
                application("Zen Browser", "Web Browser"),
            ],
        }
    }

    fn history_with(launches: &[(&str, &str, u64)]) -> LaunchHistory {
        let mut history = LaunchHistory::default();
        for (name, query, at) in launches {
            history.record(&format!("{name}.desktop"), query, *at);
        }
        history
    }

    #[test]
    fn launched_applications_lead_a_tier_and_an_empty_query() {
        let catalogue = used_catalogue();
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        let now = 1_800_000_000_000;
        assert_eq!(catalogue.search("f", &mut matcher), vec![0, 1, 2]);
        let history = history_with(&[
            ("Fonts", "", now),
            ("Firefox", "", now),
            ("Firefox", "", now),
        ]);
        assert_eq!(
            catalogue.search_ranked("f", &mut matcher, &history, now),
            vec![1, 2, 0]
        );
        assert_eq!(
            catalogue.search_ranked("", &mut matcher, &history, now),
            vec![1, 2, 0, 3]
        );
        assert_eq!(
            catalogue.search_ranked("", &mut matcher, &LaunchHistory::default(), now),
            vec![0, 1, 2, 3]
        );
    }

    #[test]
    fn a_learned_term_promotes_an_application_and_can_find_it_alone() {
        let catalogue = used_catalogue();
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        let now = 1_800_000_000_000;
        let history = history_with(&[("Zen Browser", "web", now), ("Firefox", "ff", now)]);
        // "web" matches Firefox and Zen's metadata; Zen learned it exactly, so it leads.
        assert_eq!(
            catalogue
                .search_ranked("web", &mut matcher, &history, now)
                .first(),
            Some(&3)
        );
        // "ff" is not a subsequence of Zen Browser but is a fuzzy hit for Firefox's name; the
        // learned term keeps Firefox first even against the exact name "Files" for "fi".
        assert_eq!(
            catalogue
                .search_ranked("ff", &mut matcher, &history, now)
                .first(),
            Some(&1)
        );
        let lone = history_with(&[("Fonts", "typography", now)]);
        assert_eq!(
            catalogue.search_ranked("typography", &mut matcher, &lone, now),
            vec![2]
        );
        // Expired terms stop matching.
        let later = now + 18 * 86_400_000;
        assert!(
            catalogue
                .search_ranked("typography", &mut matcher, &lone, later)
                .is_empty()
        );
    }

    #[test]
    fn exact_names_outrank_longer_prefixes_and_metadata_stays_searchable() {
        let catalogue = Catalogue {
            applications: vec![
                application("Zen Browser Nightly", "Web Browser"),
                application("Zen", "Web Browser"),
                application("Something Else", "calculator"),
            ],
        };
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        assert_eq!(catalogue.search("zen", &mut matcher).first(), Some(&1));
        assert_eq!(catalogue.search("calculator", &mut matcher), vec![2]);
    }

    #[test]
    fn hidden_user_entry_masks_system_entry_and_nested_ids_keep_precedence() -> anyhow::Result<()> {
        let temporary = std::env::temp_dir().join(format!("dojo-launcher-{}", std::process::id()));
        let user = temporary.join("user");
        let system = temporary.join("system");
        fs::create_dir_all(user.join("nested"))?;
        fs::create_dir_all(&system)?;
        fs::write(
            user.join("hidden.desktop"),
            "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\nExec=true\n",
        )?;
        fs::write(
            system.join("hidden.desktop"),
            "[Desktop Entry]\nType=Application\nName=Visible\nExec=true\n",
        )?;
        fs::write(
            user.join("nested/app.desktop"),
            "[Desktop Entry]\nType=Application\nName=Firefox\nName[fr]=Navigateur\nExec=editor %U\nStartupWMClass=EditorFixture\n",
        )?;
        fs::write(
            system.join("nested-app.desktop"),
            "[Desktop Entry]\nType=Application\nName=Wrong override\nExec=true\n",
        )?;
        let catalogue = Catalogue::from_roots(&[user, system], &["fr".into()], &["niri".into()])?;
        assert_eq!(catalogue.applications.len(), 1);
        assert_eq!(catalogue.applications[0].name, "Navigateur");
        assert_eq!(catalogue.applications[0].id, "nested-app.desktop");
        assert!(
            catalogue.applications[0]
                .last_used
                .as_deref()
                .is_some_and(|value| value.ends_with('Z'))
        );
        assert_eq!(
            catalogue.applications[0].launch_command.as_deref(),
            Some("editor")
        );
        assert_eq!(
            catalogue.applications[0].startup_wm_class.as_deref(),
            Some("EditorFixture")
        );
        fs::remove_dir_all(temporary)?;
        Ok(())
    }

    #[test]
    fn fuzzy_matching_supports_subsequences_and_unicode() {
        let catalogue = Catalogue {
            applications: vec![
                Application {
                    id: "firefox.desktop".into(),
                    name: "Firefox".into(),
                    description: String::new(),
                    path: PathBuf::new(),
                    icon: None,
                    launch_command: None,
                    startup_wm_class: None,
                    last_used: None,
                    searchable_name: Utf32String::from("Firefox"),
                    searchable: Utf32String::from("Firefox"),
                },
                Application {
                    id: "unicode.desktop".into(),
                    name: "Éditeur".into(),
                    description: String::new(),
                    path: PathBuf::new(),
                    icon: None,
                    launch_command: None,
                    startup_wm_class: None,
                    last_used: None,
                    searchable_name: Utf32String::from("Éditeur"),
                    searchable: Utf32String::from("Éditeur"),
                },
            ],
        };
        let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
        assert_eq!(catalogue.search("ffx", &mut matcher), vec![0]);
        assert_eq!(catalogue.search("edit", &mut matcher), vec![1]);
        assert!(catalogue.search("not-present", &mut matcher).is_empty());
    }

    #[test]
    #[ignore = "requires a systemd user session; starts only a temporary test script"]
    fn launch_preserves_desktop_entry_arguments_and_working_directory() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("dojo launch fixture {}", std::process::id()));
        fs::create_dir_all(&root)?;
        let script = root.join("capture arguments");
        let marker = root.join("result");
        let lifetime = root.join("lifetime");
        let survived = root.join("survived");
        fs::write(
            &script,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"${{LD_LIBRARY_PATH-unset}}\" \"$PWD\" \"$@\" > '{}'\nps -o pgid= -p $$ > '{}'\ncat /proc/$$/cgroup >> '{}'\nsleep 1\nprintf survived > '{}'\n",
                marker.display(),
                lifetime.display(),
                lifetime.display(),
                survived.display()
            ),
        )?;
        fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
        let path = root.join("fixture.desktop");
        fs::write(
            &path,
            format!(
                "[Desktop Entry]\nType=Application\nName=Dojo Launch Test\nExec=\"{}\" %c %% \"two words\"\nPath={}\n",
                script.display(),
                root.display()
            ),
        )?;
        let app = Application {
            id: "fixture.desktop".into(),
            name: "Dojo Launch Test".into(),
            description: String::new(),
            path,
            icon: None,
            launch_command: None,
            startup_wm_class: None,
            last_used: None,
            searchable_name: Utf32String::from("Dojo Launch Test"),
            searchable: Utf32String::from("test"),
        };
        let session_library_path = desktop_session_environment()?
            .into_iter()
            .find_map(|(key, value)| (key == "LD_LIBRARY_PATH").then_some(value))
            .unwrap_or_else(|| "unset".into());
        launch(&app)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let output = fs::read_to_string(&marker)?;
        assert_eq!(
            output,
            format!(
                "{session_library_path}\n{}\nDojo Launch Test\n%\ntwo words\n",
                root.display()
            )
        );
        while !survived.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(fs::read_to_string(&survived)?, "survived");
        let lifetime = fs::read_to_string(&lifetime)?;
        let own_group = Command::new("ps")
            .args(["-o", "pgid=", "-p", &std::process::id().to_string()])
            .output()?;
        assert_ne!(
            lifetime.lines().next().map(str::trim),
            Some(String::from_utf8(own_group.stdout)?.trim())
        );
        assert!(lifetime.contains(".scope") || lifetime.contains(".service"));
        assert_ne!(
            lifetime.lines().nth(1),
            fs::read_to_string("/proc/self/cgroup")?.lines().next()
        );
        fs::remove_dir_all(root)?;
        Ok(())
    }
}
