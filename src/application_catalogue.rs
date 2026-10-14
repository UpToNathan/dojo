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
        if query.is_empty() {
            return (0..self.applications.len().min(RESULT_LIMIT)).collect();
        }
        let pattern = Pattern::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let exact = Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Exact,
            false,
        );
        let prefix = Atom::new(
            query,
            CaseMatching::Smart,
            Normalization::Smart,
            AtomKind::Prefix,
            false,
        );
        let mut scores = self
            .applications
            .iter()
            .enumerate()
            .filter_map(|(index, app)| {
                if let Some(score) = pattern.score(app.searchable_name.slice(..), matcher) {
                    let tier = if exact
                        .score(app.searchable_name.slice(..), matcher)
                        .is_some()
                    {
                        0
                    } else if prefix
                        .score(app.searchable_name.slice(..), matcher)
                        .is_some()
                    {
                        1
                    } else {
                        2
                    };
                    Some((index, tier, score))
                } else {
                    pattern
                        .score(app.searchable.slice(..), matcher)
                        .map(|score| (index, 3, score))
                }
            })
            .collect::<Vec<_>>();
        scores.sort_unstable_by(
            |(left, left_tier, left_score), (right, right_tier, right_score)| {
                left_tier
                    .cmp(right_tier)
                    .then_with(|| right_score.cmp(left_score))
                    .then_with(|| left.cmp(right))
            },
        );
        scores.truncate(RESULT_LIMIT);
        scores.into_iter().map(|(index, _, _)| index).collect()
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
