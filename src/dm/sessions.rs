//! The Wayland sessions the login screen offers: desktop entries in a root-owned
//! directory, parsed into validated [`Session`]s. The greeter only ever names an entry by
//! id; the `Exec` it runs comes from here, never from the greeter.
//!
//! [`parse`] is pure. [`Catalog::load`] is the boundary: it refuses a directory or an
//! entry that is not root's alone to write, reads at most [`MAX_ENTRY_BYTES`] of each,
//! and drops what does not parse.

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};

use super::launch::{MAX_DESKTOP_NAME, MAX_DESKTOP_NAMES, MAX_EXEC, Start};
use super::protocol::{MAX_NAME, MAX_SESSIONS, SessionEntry, SessionId};
use super::session::SESSION_PATH;
use super::vt::Vt;

pub const DIRECTORY: &str = "/usr/share/wayland-sessions";
pub const MAX_ENTRY_BYTES: u64 = 16 * 1024;

/// Why a desktop entry is not offered.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rejection {
    NotUtf8,
    /// No `[Desktop Entry]` group, or a key line that is not `Key=value`.
    Malformed,
    /// A key this module reads appears twice.
    Duplicate(&'static str),
    Missing(&'static str),
    Invalid(&'static str),
    /// `Hidden=true` or `NoDisplay=true`.
    NotListed,
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Rejection::NotUtf8 => f.write_str("not UTF-8"),
            Rejection::Malformed => f.write_str("not a desktop entry"),
            Rejection::Duplicate(key) => write!(f, "{key} appears twice"),
            Rejection::Missing(key) => write!(f, "no {key}"),
            Rejection::Invalid(key) => write!(f, "invalid {key}"),
            Rejection::NotListed => f.write_str("hidden"),
        }
    }
}

/// What an entry says, before the host is asked whether its program exists.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Parsed {
    pub name: String,
    /// The `Exec` line with its field codes removed: one line for the user's shell.
    pub exec: String,
    pub desktop_names: Vec<String>,
    pub try_exec: Option<String>,
}

/// A session the display manager has agreed to start.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Session {
    pub entry: SessionEntry,
    exec: String,
    desktop_names: Vec<String>,
}

impl Session {
    /// The helper's instruction for this session on this console.
    pub fn start(&self, vt: &Vt) -> Start {
        Start {
            exec: self.exec.clone(),
            desktop: self.entry.id.clone(),
            desktop_names: self.desktop_names.clone(),
            vtnr: vt.number(),
            tty: vt.tty(),
        }
    }

    pub fn exec(&self) -> &str {
        &self.exec
    }
}

#[derive(Default)]
struct Keys {
    name: Option<String>,
    exec: Option<String>,
    try_exec: Option<String>,
    desktop_names: Option<String>,
    hidden: Option<String>,
    no_display: Option<String>,
    kind: Option<String>,
}

/// The `[Desktop Entry]` group of one file. Localized keys (`Name[de]`) and every key this
/// module does not read are ignored; the first line that is neither blank, a comment, a
/// group header nor `Key=value` makes the whole entry garbage.
pub fn parse(text: &str) -> Result<Parsed, Rejection> {
    let mut keys = Keys::default();
    let mut in_group = false;
    let mut seen_group = false;
    for line in text.lines() {
        let line = line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(header) = line.strip_prefix('[') {
            let name = header.strip_suffix(']').ok_or(Rejection::Malformed)?;
            if in_group {
                break;
            }
            in_group = name == "Desktop Entry";
            seen_group |= in_group;
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(Rejection::Malformed)?;
        if !in_group {
            return Err(Rejection::Malformed);
        }
        let key = key.trim_end();
        let slot = match key {
            "Name" => &mut keys.name,
            "Exec" => &mut keys.exec,
            "TryExec" => &mut keys.try_exec,
            "DesktopNames" => &mut keys.desktop_names,
            "Hidden" => &mut keys.hidden,
            "NoDisplay" => &mut keys.no_display,
            "Type" => &mut keys.kind,
            _ => continue,
        };
        if slot.replace(value.trim().to_owned()).is_some() {
            return Err(Rejection::Duplicate(known_key(key)));
        }
    }
    if !seen_group {
        return Err(Rejection::Malformed);
    }
    for flag in [&keys.hidden, &keys.no_display] {
        match flag.as_deref() {
            None | Some("false") => {}
            Some("true") => return Err(Rejection::NotListed),
            Some(_) => return Err(Rejection::Invalid("Hidden")),
        }
    }
    if keys.kind.as_deref().is_some_and(|kind| kind != "Application") {
        return Err(Rejection::Invalid("Type"));
    }
    let name = clean_name(keys.name.ok_or(Rejection::Missing("Name"))?)?;
    let exec = strip_exec(&keys.exec.ok_or(Rejection::Missing("Exec"))?)?;
    let desktop_names = match keys.desktop_names {
        Some(list) => desktop_names(&list)?,
        None => Vec::new(),
    };
    let try_exec = keys.try_exec.filter(|value| !value.is_empty());
    if try_exec.as_ref().is_some_and(|value| {
        value.chars().any(char::is_control) || value.len() > MAX_EXEC
    }) {
        return Err(Rejection::Invalid("TryExec"));
    }
    Ok(Parsed {
        name,
        exec,
        desktop_names,
        try_exec,
    })
}

fn known_key(key: &str) -> &'static str {
    ["Name", "Exec", "TryExec", "DesktopNames", "Hidden", "NoDisplay", "Type"]
        .into_iter()
        .find(|known| *known == key)
        .unwrap_or("key")
}

fn clean_name(raw: String) -> Result<String, Rejection> {
    let name = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() || name.len() > MAX_NAME || name.chars().any(char::is_control) {
        return Err(Rejection::Invalid("Name"));
    }
    Ok(name)
}

/// `%f`, `%F`, `%u`, `%U` and the deprecated codes expand to nothing in a session, `%%` is
/// a percent sign, anything else after a percent sign is not a field code at all.
fn strip_exec(raw: &str) -> Result<String, Rejection> {
    let invalid = Rejection::Invalid("Exec");
    let mut unescaped = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            unescaped.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => unescaped.push(' '),
            Some('\\') => unescaped.push('\\'),
            Some('n' | 't' | 'r') | None => return Err(invalid),
            Some(other) => {
                unescaped.push('\\');
                unescaped.push(other);
            }
        }
    }
    let mut stripped = String::with_capacity(unescaped.len());
    let mut chars = unescaped.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            stripped.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => stripped.push('%'),
            Some('f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm') => {}
            _ => return Err(invalid),
        }
    }
    let exec = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    if exec.is_empty() || exec.len() > MAX_EXEC || exec.chars().any(char::is_control) {
        return Err(invalid);
    }
    Ok(exec)
}

/// `a;b;` as the specification writes lists. Each name is one word of `XDG_CURRENT_DESKTOP`.
fn desktop_names(list: &str) -> Result<Vec<String>, Rejection> {
    let invalid = Rejection::Invalid("DesktopNames");
    let mut names = Vec::new();
    for name in list.split(';').filter(|name| !name.is_empty()) {
        let valid = name.len() <= MAX_DESKTOP_NAME
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.'));
        if !valid || names.len() == MAX_DESKTOP_NAMES {
            return Err(invalid);
        }
        names.push(name.to_owned());
    }
    Ok(names)
}

/// The places a bare `TryExec` is looked up: the session's own `PATH`.
pub fn try_exec_candidates(try_exec: &str) -> Vec<PathBuf> {
    if try_exec.starts_with('/') {
        return vec![PathBuf::from(try_exec)];
    }
    if try_exec.contains('/') {
        return Vec::new();
    }
    SESSION_PATH
        .split(':')
        .map(|directory| Path::new(directory).join(try_exec))
        .collect()
}

/// What the file system says about one entry, so the trust rule is a pure function.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Facts {
    pub regular: bool,
    pub directory: bool,
    pub uid: u32,
    pub mode: u32,
}

impl Facts {
    fn of(metadata: &fs::Metadata) -> Facts {
        let kind = metadata.file_type();
        Facts {
            regular: kind.is_file(),
            directory: kind.is_dir(),
            uid: metadata.uid(),
            mode: metadata.mode(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Distrust {
    NotRegular,
    NotDirectory,
    Owner,
    Writable,
}

impl fmt::Display for Distrust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Distrust::NotRegular => "not a regular file",
            Distrust::NotDirectory => "not a directory",
            Distrust::Owner => "not owned by root",
            Distrust::Writable => "writable by others",
        })
    }
}

/// An entry is trusted when it is a regular file that only `owner` can change. The
/// directory has the same rule, so nobody else can swap an entry in.
pub fn trusted(facts: Facts, directory: bool, owner: u32) -> Result<(), Distrust> {
    if directory && !facts.directory {
        return Err(Distrust::NotDirectory);
    }
    if !directory && !facts.regular {
        return Err(Distrust::NotRegular);
    }
    if facts.uid != owner {
        return Err(Distrust::Owner);
    }
    if facts.mode & 0o022 != 0 {
        return Err(Distrust::Writable);
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Catalog {
    sessions: Vec<Session>,
}

/// One entry that was left out, for the log.
#[derive(Debug)]
pub struct Skipped {
    pub file: String,
    pub reason: String,
}

impl Catalog {
    pub fn from_sessions(mut sessions: Vec<Session>) -> Catalog {
        sessions.sort_by(|a, b| a.entry.id.cmp(&b.entry.id));
        sessions.truncate(MAX_SESSIONS);
        Catalog { sessions }
    }

    /// The offered sessions in id order. `owner` is root's uid in a real service.
    /// `available` answers whether a `TryExec` candidate is an executable file.
    pub fn load(
        directory: &Path,
        owner: u32,
        available: impl Fn(&Path) -> bool,
    ) -> (Catalog, Vec<Skipped>) {
        let mut skipped = Vec::new();
        let mut note = |file: &str, reason: &dyn fmt::Display| {
            skipped.push(Skipped {
                file: file.to_owned(),
                reason: reason.to_string(),
            });
        };
        let trusted_directory = fs::symlink_metadata(directory)
            .map_err(|error| error.to_string())
            .and_then(|metadata| {
                trusted(Facts::of(&metadata), true, owner).map_err(|why| why.to_string())
            });
        if let Err(why) = trusted_directory {
            note(&directory.display().to_string(), &why);
            return (Catalog::default(), skipped);
        }
        let mut names: Vec<String> = fs::read_dir(directory)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .filter(|name| name.ends_with(".desktop"))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        let mut sessions = Vec::new();
        for file in names {
            let Some(id) = file.strip_suffix(".desktop").and_then(SessionId::parse) else {
                note(&file, &"not a usable session id");
                continue;
            };
            let text = match read_entry(&directory.join(&file), owner) {
                Ok(text) => text,
                Err(why) => {
                    note(&file, &why);
                    continue;
                }
            };
            let parsed = match parse(&text) {
                Ok(parsed) => parsed,
                Err(Rejection::NotListed) => continue,
                Err(why) => {
                    note(&file, &why);
                    continue;
                }
            };
            if let Some(program) = &parsed.try_exec
                && !try_exec_candidates(program).iter().any(|path| available(path))
            {
                note(&file, &format!("{program} is not installed"));
                continue;
            }
            sessions.push(Session {
                entry: SessionEntry {
                    id,
                    name: parsed.name,
                },
                exec: parsed.exec,
                desktop_names: parsed.desktop_names,
            });
        }
        (Catalog::from_sessions(sessions), skipped)
    }

    pub fn entries(&self) -> Vec<SessionEntry> {
        self.sessions.iter().map(|s| s.entry.clone()).collect()
    }

    /// `None` for an id that is not an offered entry, whatever else exists on disk.
    pub fn resolve(&self, id: &SessionId) -> Option<&Session> {
        self.sessions.iter().find(|session| &session.entry.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

#[derive(Debug)]
enum ReadError {
    Distrust(Distrust),
    TooLarge,
    Io(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::Distrust(why) => why.fmt(f),
            ReadError::TooLarge => write!(f, "larger than {MAX_ENTRY_BYTES} bytes"),
            ReadError::Io(error) => error.fmt(f),
        }
    }
}

/// Opens without following a link and without waiting on a FIFO, then judges the file
/// that was opened rather than the path, so nothing can be swapped in between.
fn read_entry(path: &Path, owner: u32) -> Result<String, ReadError> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)
        .map_err(ReadError::Io)?;
    let metadata = file.metadata().map_err(ReadError::Io)?;
    trusted(Facts::of(&metadata), false, owner).map_err(ReadError::Distrust)?;
    read_capped(file)
}

fn read_capped(file: File) -> Result<String, ReadError> {
    let mut bytes = Vec::new();
    file.take(MAX_ENTRY_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(ReadError::Io)?;
    if bytes.len() as u64 > MAX_ENTRY_BYTES {
        return Err(ReadError::TooLarge);
    }
    String::from_utf8(bytes).map_err(|_| ReadError::Io(io::ErrorKind::InvalidData.into()))
}

#[cfg(test)]
mod tests;
