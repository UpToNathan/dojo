//! 11 Services: systemd user units. See .scratch/launcher-features/issues/ for the ticket.
//!
//! Pure logic lives here; the launcher wires it in. Keep I/O behind small functions so the rest is testable.
//!
//! Everything goes through `systemctl --user` and `journalctl --user`, so it needs no privileges
//! and never asks for a password. The pieces:
//!
//! * [`list`] reads `list-units --type=service --all --output=json` (systemd 248 and later; the
//!   plain-text table is parsed when the JSON form is refused) and adds installed unit files that
//!   are not loaded, which are services the user could start.
//! * [`search`] and [`order`] rank them: failed units first, then running ones.
//! * [`action_argv`] builds the command for start, stop, restart and reset-failed, and [`run`]
//!   executes it. [`recent_log`] returns the last lines of a unit's journal.
//!
//! Unit names reach the command line, so [`validate_unit_name`] runs before any argv is built:
//! no leading `-`, a `.service`, `.socket` or `.timer` suffix, only characters systemd allows in
//! a name, no glob characters, and the name follows a `--` so it can never be read as an option.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Matcher, Utf32String};
use serde::Deserialize;

const LIST_TIMEOUT: Duration = Duration::from_secs(5);
const LOG_TIMEOUT: Duration = Duration::from_secs(5);
/// Starting or stopping waits for the job, which can take a while for a slow unit.
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);
const OUTPUT_LIMIT: u64 = 4 * 1024 * 1024;
const MAX_UNIT_NAME: usize = 256;
const LOG_LINES: usize = 20;
const LOG_LINE_CHARS: usize = 500;
const UNIT_SUFFIXES: [&str; 3] = [".service", ".socket", ".timer"];

/// One user unit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServiceUnit {
    /// Full unit name with suffix, exactly as systemd reports it (escapes such as `\x2d` intact).
    pub name: String,
    pub description: String,
    /// `loaded`, `not-found`, `masked`, ... or `not-loaded` for an installed unit file that
    /// systemd has not loaded.
    pub load: String,
    /// `active`, `inactive`, `failed`, `activating`, ...
    pub active: String,
    /// `running`, `dead`, `exited`, `failed`, ...
    pub sub: String,
    /// The unit-file state (`enabled`, `disabled`, `static`, `generated`, ...) when known.
    pub file_state: Option<String>,
}

impl ServiceUnit {
    pub fn is_failed(&self) -> bool {
        self.active == "failed" || self.sub == "failed"
    }

    pub fn is_active(&self) -> bool {
        matches!(self.active.as_str(), "active" | "activating" | "reloading")
    }

    /// The name for people: the suffix kept, systemd's `\xNN` escapes turned back into the
    /// characters they stand for.
    pub fn display_name(&self) -> String {
        unescape_unit_name(&self.name)
    }

    /// The display name without `.service`.
    pub fn short_name(&self) -> String {
        let name = self.display_name();
        name.strip_suffix(".service")
            .map(str::to_owned)
            .unwrap_or(name)
    }
}

// ---------------------------------------------------------------------------------------------
// Names

/// Why a unit name was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NameError {
    Empty,
    TooLong,
    /// A name starting with `-` would be read as an option.
    LeadingDash,
    BadSuffix,
    /// A character outside `[A-Za-z0-9:_.@\\-]`.
    BadCharacter(char),
}

impl std::fmt::Display for NameError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("the unit name is empty"),
            Self::TooLong => write!(formatter, "the unit name is over {MAX_UNIT_NAME} bytes"),
            Self::LeadingDash => formatter.write_str("a unit name cannot start with '-'"),
            Self::BadSuffix => {
                formatter.write_str("the unit name must end in .service, .socket or .timer")
            }
            Self::BadCharacter(character) => {
                write!(formatter, "{character:?} is not allowed in a unit name")
            }
        }
    }
}

impl std::error::Error for NameError {}

/// Accepts names of user units the launcher manages. `\` is allowed because systemd escapes
/// awkward characters that way (`app-gnome\x2dkeyring.service`); whitespace, glob characters
/// (`*?[`), `/`, `%` and control characters are not.
pub fn validate_unit_name(name: &str) -> Result<(), NameError> {
    if name.is_empty() {
        return Err(NameError::Empty);
    }
    if name.len() > MAX_UNIT_NAME {
        return Err(NameError::TooLong);
    }
    if name.starts_with('-') {
        return Err(NameError::LeadingDash);
    }
    if let Some(bad) = name
        .chars()
        .find(|c| !(c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '.' | '@' | '\\' | '-')))
    {
        return Err(NameError::BadCharacter(bad));
    }
    let suffix = UNIT_SUFFIXES.iter().find(|suffix| name.ends_with(*suffix));
    match suffix {
        Some(suffix) if name.len() > suffix.len() => Ok(()),
        _ => Err(NameError::BadSuffix),
    }
}

/// Turns systemd's `\xNN` escapes back into characters. Anything else, including a malformed
/// escape, is kept as written.
pub fn unescape_unit_name(name: &str) -> String {
    let bytes = name.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\'
            && bytes.get(index + 1) == Some(&b'x')
            && let Some(pair) = bytes.get(index + 2..index + 4)
            && let Ok(text) = std::str::from_utf8(pair)
            && let Ok(value) = u8::from_str_radix(text, 16)
        {
            out.push(value);
            index += 4;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------------------------
// Parsing

#[derive(Deserialize)]
struct JsonUnit {
    unit: String,
    #[serde(default)]
    load: String,
    #[serde(default)]
    active: String,
    #[serde(default)]
    sub: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
struct JsonUnitFile {
    unit_file: String,
    #[serde(default)]
    state: Option<String>,
}

fn is_service(name: &str) -> bool {
    name.ends_with(".service") && validate_unit_name(name).is_ok()
}

fn unit_from(json: JsonUnit) -> ServiceUnit {
    ServiceUnit {
        name: json.unit,
        description: json.description,
        load: json.load,
        active: json.active,
        sub: json.sub,
        file_state: None,
    }
}

/// Parses `systemctl list-units --output=json`. Units whose names the engine would refuse to
/// act on are left out, so every listed unit can be passed to [`action_argv`].
pub fn parse_units_json(text: &str) -> Result<Vec<ServiceUnit>> {
    let units: Vec<JsonUnit> =
        serde_json::from_str(text).context("parsing systemctl list-units JSON")?;
    Ok(units
        .into_iter()
        .filter(|unit| is_service(&unit.unit))
        .map(unit_from)
        .collect())
}

/// Parses the table from `systemctl list-units --plain --no-legend --no-pager`: four words
/// (unit, load, active, sub) and the description as the rest of the line. A leading bullet
/// (`●`, `*`) that older systemd prints for problem units is ignored.
pub fn parse_units_plain(text: &str) -> Vec<ServiceUnit> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let line = line
                .strip_prefix('●')
                .or_else(|| line.strip_prefix('*'))
                .or_else(|| line.strip_prefix('○'))
                .unwrap_or(line)
                .trim_start();
            // Columns are padded with runs of spaces, so peel one word at a time.
            let mut rest = line;
            let mut word = || {
                let (word, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
                rest = tail.trim_start();
                (!word.is_empty()).then_some(word)
            };
            let name = word()?;
            let load = word()?;
            let active = word()?;
            let sub = word()?;
            let description = rest.trim();
            is_service(name).then(|| ServiceUnit {
                name: name.to_owned(),
                description: description.to_owned(),
                load: load.to_owned(),
                active: active.to_owned(),
                sub: sub.to_owned(),
                file_state: None,
            })
        })
        .collect()
}

/// Parses `systemctl list-unit-files --output=json` into `(name, state)` pairs for services.
pub fn parse_unit_files_json(text: &str) -> Result<Vec<(String, Option<String>)>> {
    let files: Vec<JsonUnitFile> =
        serde_json::from_str(text).context("parsing systemctl list-unit-files JSON")?;
    Ok(files
        .into_iter()
        .filter(|file| is_service(&file.unit_file))
        .map(|file| (file.unit_file, file.state))
        .collect())
}

/// Combines loaded units with installed unit files: units that are loaded gain their file state,
/// and installed services that are not loaded appear as inactive, `not-loaded` units. Template
/// files (`name@.service`) cannot be started without an instance and are skipped.
pub fn merge_unit_files(
    mut units: Vec<ServiceUnit>,
    files: Vec<(String, Option<String>)>,
) -> Vec<ServiceUnit> {
    for (file, state) in files {
        if file.ends_with("@.service") {
            continue;
        }
        match units.iter_mut().find(|unit| unit.name == file) {
            Some(unit) => unit.file_state = state,
            None => units.push(ServiceUnit {
                name: file,
                description: String::new(),
                load: "not-loaded".to_owned(),
                active: "inactive".to_owned(),
                sub: "dead".to_owned(),
                file_state: state,
            }),
        }
    }
    units
}

// ---------------------------------------------------------------------------------------------
// Ordering and search

fn status_rank(unit: &ServiceUnit) -> u8 {
    if unit.is_failed() {
        0
    } else if unit.is_active() {
        1
    } else if unit.load == "loaded" {
        2
    } else {
        3
    }
}

fn default_order(left: &ServiceUnit, right: &ServiceUnit) -> std::cmp::Ordering {
    status_rank(left)
        .cmp(&status_rank(right))
        .then_with(|| left.display_name().cmp(&right.display_name()))
}

/// Indices of every unit: failed first, then active, then other loaded units, then the rest,
/// each by name.
pub fn order(units: &[ServiceUnit]) -> Vec<usize> {
    let mut indices = (0..units.len()).collect::<Vec<_>>();
    indices.sort_by(|&left, &right| default_order(&units[left], &units[right]));
    indices
}

/// Units matching `query`, best first, as indices. An empty query is [`order`]. Matches fall
/// into tiers: the exact name (with or without `.service`), a name prefix, a fuzzy name match,
/// then a fuzzy description match. Within a tier failed units come first, then active ones, then
/// by name.
pub fn search(units: &[ServiceUnit], query: &str, matcher: &mut Matcher) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return order(units);
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
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

    let mut matched = Vec::new();
    for (index, unit) in units.iter().enumerate() {
        let full = Utf32String::from(unit.display_name().as_str());
        let short = Utf32String::from(unit.short_name().as_str());
        let (tier, score) = if exact.score(full.slice(..), matcher).is_some()
            || exact.score(short.slice(..), matcher).is_some()
        {
            (0, 0)
        } else if prefix.score(full.slice(..), matcher).is_some() {
            (1, 0)
        } else if let Some(score) = [&full, &short]
            .into_iter()
            .filter_map(|name| pattern.score(name.slice(..), matcher))
            .max()
        {
            (2, score)
        } else if let Some(score) = pattern.score(
            Utf32String::from(unit.description.as_str()).slice(..),
            matcher,
        ) {
            (3, score)
        } else {
            continue;
        };
        matched.push((tier, score, index));
    }
    // Within a fuzzy tier the better match comes first; units that match equally well keep the
    // default order (failed, active, then by name).
    matched.sort_by(
        |&(left_tier, left_score, left), &(right_tier, right_score, right)| {
            left_tier
                .cmp(&right_tier)
                .then_with(|| right_score.cmp(&left_score))
                .then_with(|| default_order(&units[left], &units[right]))
        },
    );
    matched.into_iter().map(|(_, _, index)| index).collect()
}

// ---------------------------------------------------------------------------------------------
// Actions and argv

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    /// Clears a unit's failed state.
    ResetFailed,
}

impl Action {
    fn verb(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::Stop => "stop",
            Self::Restart => "restart",
            Self::ResetFailed => "reset-failed",
        }
    }
}

/// `systemctl --user <verb> --no-ask-password -- <unit>` for a validated unit name.
pub fn action_argv(action: Action, unit: &str) -> Result<Vec<String>, NameError> {
    validate_unit_name(unit)?;
    Ok([
        "systemctl",
        "--user",
        action.verb(),
        "--no-ask-password",
        "--",
        unit,
    ]
    .map(str::to_owned)
    .to_vec())
}

/// `journalctl --user -u <unit> -n 20 --no-pager -o cat` for a validated unit name.
pub fn log_argv(unit: &str) -> Result<Vec<String>, NameError> {
    validate_unit_name(unit)?;
    Ok([
        "journalctl",
        "--user",
        "-u",
        unit,
        "-n",
        &LOG_LINES.to_string(),
        "--no-pager",
        "-o",
        "cat",
    ]
    .map(str::to_owned)
    .to_vec())
}

fn list_units_argv(json: bool) -> Vec<String> {
    let mut argv = [
        "systemctl",
        "--user",
        "list-units",
        "--type=service",
        "--all",
    ]
    .map(str::to_owned)
    .to_vec();
    if json {
        argv.push("--output=json".to_owned());
    } else {
        argv.extend(["--plain", "--no-legend"].map(str::to_owned));
    }
    argv.push("--no-pager".to_owned());
    argv
}

fn list_unit_files_argv() -> Vec<String> {
    [
        "systemctl",
        "--user",
        "list-unit-files",
        "--type=service",
        "--output=json",
        "--no-pager",
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Cleans journal text for display: control characters (including escape sequences' ESC) become
/// nothing, tabs become spaces, and long lines are cut.
pub fn clean_log_line(line: &str) -> String {
    let cleaned = line
        .chars()
        .filter_map(|c| match c {
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect::<String>();
    if cleaned.chars().count() > LOG_LINE_CHARS {
        let mut cut = cleaned.chars().take(LOG_LINE_CHARS).collect::<String>();
        cut.push('…');
        cut
    } else {
        cleaned
    }
}

/// The journal lines from `-o cat` output: cleaned, without the `-- No entries --` notices.
pub fn parse_log(text: &str) -> Vec<String> {
    text.lines()
        .map(clean_log_line)
        .filter(|line| !line.starts_with("-- ") || !line.ends_with(" --"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Running commands

#[derive(Debug)]
struct CommandOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Runs `argv` with a deadline, killing it when the deadline passes.
fn run_command(argv: &[String], timeout: Duration) -> Result<CommandOutput> {
    let (program, arguments) = argv.split_first().context("an empty command")?;
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {program}"))?;
    let read = |stream: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(stream) = stream {
                let _ = stream.take(OUTPUT_LIMIT).read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = read(child.stdout.take().map(|s| Box::new(s) as _));
    let stderr = read(child.stderr.take().map(|s| Box::new(s) as _));
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait().context("waiting for the command")? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{program} did not finish within {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    let text = |handle: std::thread::JoinHandle<Vec<u8>>| {
        String::from_utf8_lossy(&handle.join().unwrap_or_default()).into_owned()
    };
    Ok(CommandOutput {
        success: status.success(),
        stdout: text(stdout),
        stderr: text(stderr),
    })
}

type Runner<'a> = &'a dyn Fn(&[String], Duration) -> Result<CommandOutput>;

fn failure(what: &str, output: &CommandOutput) -> anyhow::Error {
    let detail = output.stderr.trim();
    if detail.is_empty() {
        anyhow::anyhow!("{what} failed")
    } else {
        anyhow::anyhow!("{what} failed: {}", clean_log_line(detail))
    }
}

fn list_with(runner: Runner<'_>) -> Result<Vec<ServiceUnit>> {
    let json = runner(&list_units_argv(true), LIST_TIMEOUT)?;
    let units = if json.success {
        parse_units_json(&json.stdout)?
    } else {
        // systemd before 248 has no --output=json for list-units.
        let plain = runner(&list_units_argv(false), LIST_TIMEOUT)?;
        if !plain.success {
            return Err(failure("listing user services", &plain));
        }
        parse_units_plain(&plain.stdout)
    };
    // Installed-but-unloaded services are a bonus; losing them must not lose the list.
    let files = runner(&list_unit_files_argv(), LIST_TIMEOUT)
        .ok()
        .filter(|output| output.success)
        .and_then(|output| parse_unit_files_json(&output.stdout).ok())
        .unwrap_or_default();
    Ok(merge_unit_files(units, files))
}

/// Every user service systemd knows, loaded or installed, in no particular order (see
/// [`order`] and [`search`]). Fails when no user manager is reachable.
pub fn list() -> Result<Vec<ServiceUnit>> {
    list_with(&run_command)
}

/// Runs an action on a unit and waits for systemd to finish the job.
pub fn run(action: Action, unit: &str) -> Result<()> {
    run_with(&run_command, action, unit)
}

fn run_with(runner: Runner<'_>, action: Action, unit: &str) -> Result<()> {
    let argv = action_argv(action, unit)?;
    let output = runner(&argv, ACTION_TIMEOUT)?;
    if output.success {
        Ok(())
    } else {
        Err(failure(&format!("{} {unit}", action.verb()), &output))
    }
}

/// The last 20 journal lines of a unit, oldest first. Empty when the unit has logged nothing.
pub fn recent_log(unit: &str) -> Result<Vec<String>> {
    recent_log_with(&run_command, unit)
}

fn recent_log_with(runner: Runner<'_>, unit: &str) -> Result<Vec<String>> {
    let argv = log_argv(unit)?;
    let output = runner(&argv, LOG_TIMEOUT)?;
    if output.success {
        Ok(parse_log(&output.stdout))
    } else {
        Err(failure(&format!("reading the journal of {unit}"), &output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const JSON: &str = r#"[
      {"unit":"app-dojo@autostart.service","load":"loaded","active":"active","sub":"running","description":"Dojo"},
      {"unit":"app-gnome\\x2dkeyring\\x2dsecrets@autostart.service","load":"loaded","active":"inactive","sub":"dead","description":"Secret Storage Service"},
      {"unit":"clipvault-history.service","load":"loaded","active":"failed","sub":"failed","description":"Store Wayland clipboard entries"},
      {"unit":"missing.service","load":"not-found","active":"inactive","sub":"dead","description":"missing.service"},
      {"unit":"weird name.service","load":"loaded","active":"active","sub":"running","description":"has a space"},
      {"unit":"--evil.service","load":"loaded","active":"active","sub":"running","description":"option"},
      {"unit":"pipewire.service","load":"loaded","active":"active","sub":"running","description":"PipeWire Multimedia Service","extra":1}
    ]"#;

    const FILES: &str = r#"[
      {"unit_file":"pipewire.service","state":"disabled","preset":"enabled"},
      {"unit_file":"syncthing.service","state":"enabled","preset":null},
      {"unit_file":"foo@.service","state":"static","preset":null},
      {"unit_file":"dojo.target","state":"static","preset":null}
    ]"#;

    fn units() -> Vec<ServiceUnit> {
        let files = parse_unit_files_json(FILES).unwrap();
        merge_unit_files(parse_units_json(JSON).unwrap(), files)
    }

    fn matcher() -> Matcher {
        Matcher::new(nucleo_matcher::Config::DEFAULT)
    }

    #[test]
    fn json_parsing_keeps_valid_services_and_drops_unsafe_names() {
        let parsed = parse_units_json(JSON).unwrap();
        let names = parsed.iter().map(|u| u.name.as_str()).collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "app-dojo@autostart.service",
                "app-gnome\\x2dkeyring\\x2dsecrets@autostart.service",
                "clipvault-history.service",
                "missing.service",
                "pipewire.service",
            ]
        );
        let failed = parsed
            .iter()
            .find(|u| u.name == "clipvault-history.service")
            .unwrap();
        assert!(failed.is_failed() && !failed.is_active());
        assert_eq!(failed.description, "Store Wayland clipboard entries");
        assert_eq!(parsed[0].load, "loaded");
        assert_eq!(parsed[0].sub, "running");
        assert!(parsed[0].is_active());
    }

    #[test]
    fn json_parsing_rejects_garbage_and_tolerates_empty_lists() {
        assert!(parse_units_json("not json").is_err());
        assert!(parse_units_json("{}").is_err());
        assert!(parse_units_json("[]").unwrap().is_empty());
    }

    #[test]
    fn plain_parsing_handles_descriptions_bullets_and_junk() {
        let text = "\
  app-dojo@autostart.service loaded active   running Dojo\n\
● clipvault-history.service loaded failed   failed  Store Wayland clipboard entries\n\
  pipewire.service           loaded active   running PipeWire Multimedia Service\n\
  no-description.service     loaded inactive dead\n\
  dojo.target                loaded active   active  not a service\n\
\n\
  short.service loaded\n";
        let parsed = parse_units_plain(text);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[1].name, "clipvault-history.service");
        assert!(parsed[1].is_failed());
        assert_eq!(parsed[2].description, "PipeWire Multimedia Service");
        assert_eq!(parsed[3].description, "");
    }

    #[test]
    fn unit_files_add_state_and_unloaded_services() {
        let merged = units();
        let pipewire = merged
            .iter()
            .find(|u| u.name == "pipewire.service")
            .unwrap();
        assert_eq!(pipewire.file_state.as_deref(), Some("disabled"));
        assert!(pipewire.is_active());
        let syncthing = merged
            .iter()
            .find(|u| u.name == "syncthing.service")
            .unwrap();
        assert_eq!(syncthing.load, "not-loaded");
        assert!(!syncthing.is_active() && !syncthing.is_failed());
        assert_eq!(syncthing.file_state.as_deref(), Some("enabled"));
        assert!(
            merged
                .iter()
                .all(|u| u.name != "foo@.service" && u.name != "dojo.target")
        );
        assert_eq!(merged.len(), 6);
    }

    #[test]
    fn valid_names_pass_and_dangerous_names_do_not() {
        for good in [
            "a.service",
            "app-dojo@autostart.service",
            "dbus-:1.2-org.example@0.service",
            "app-gnome\\x2dkeyring\\x2dsecrets@autostart.service",
            "backup.timer",
            "dbus.socket",
            "UPPER_case-1.service",
        ] {
            assert_eq!(validate_unit_name(good), Ok(()), "{good}");
        }
        assert_eq!(validate_unit_name(""), Err(NameError::Empty));
        assert_eq!(
            validate_unit_name("-x.service"),
            Err(NameError::LeadingDash)
        );
        assert_eq!(
            validate_unit_name("--user.service"),
            Err(NameError::LeadingDash)
        );
        assert_eq!(validate_unit_name("a.target"), Err(NameError::BadSuffix));
        assert_eq!(validate_unit_name("a"), Err(NameError::BadSuffix));
        assert_eq!(validate_unit_name(".service"), Err(NameError::BadSuffix));
        assert_eq!(
            validate_unit_name("a.service "),
            Err(NameError::BadCharacter(' '))
        );
        assert_eq!(
            validate_unit_name("a b.service"),
            Err(NameError::BadCharacter(' '))
        );
        assert_eq!(
            validate_unit_name("a;b.service"),
            Err(NameError::BadCharacter(';'))
        );
        assert_eq!(
            validate_unit_name("a\nb.service"),
            Err(NameError::BadCharacter('\n'))
        );
        assert_eq!(
            validate_unit_name("a\0.service"),
            Err(NameError::BadCharacter('\0'))
        );
        assert_eq!(
            validate_unit_name("a*.service"),
            Err(NameError::BadCharacter('*'))
        );
        assert_eq!(
            validate_unit_name("a?.service"),
            Err(NameError::BadCharacter('?'))
        );
        assert_eq!(
            validate_unit_name("a[1].service"),
            Err(NameError::BadCharacter('['))
        );
        assert_eq!(
            validate_unit_name("../a.service"),
            Err(NameError::BadCharacter('/'))
        );
        assert_eq!(
            validate_unit_name("a%i.service"),
            Err(NameError::BadCharacter('%'))
        );
        assert_eq!(
            validate_unit_name("é.service"),
            Err(NameError::BadCharacter('é'))
        );
        assert_eq!(
            validate_unit_name(&format!("{}.service", "a".repeat(300))),
            Err(NameError::TooLong)
        );
    }

    #[test]
    fn action_argv_puts_the_unit_after_a_double_dash() {
        assert_eq!(
            action_argv(Action::Restart, "pipewire.service").unwrap(),
            [
                "systemctl",
                "--user",
                "restart",
                "--no-ask-password",
                "--",
                "pipewire.service"
            ]
        );
        for (action, verb) in [
            (Action::Start, "start"),
            (Action::Stop, "stop"),
            (Action::ResetFailed, "reset-failed"),
        ] {
            let argv = action_argv(action, "a.service").unwrap();
            assert_eq!(argv[2], verb);
            assert_eq!(argv[argv.len() - 2], "--");
        }
        assert_eq!(
            action_argv(Action::Stop, "--now.service"),
            Err(NameError::LeadingDash)
        );
        assert_eq!(
            action_argv(Action::Stop, "a.service --now"),
            Err(NameError::BadCharacter(' '))
        );
    }

    #[test]
    fn log_argv_matches_the_documented_command() {
        assert_eq!(
            log_argv("pipewire.service").unwrap(),
            [
                "journalctl",
                "--user",
                "-u",
                "pipewire.service",
                "-n",
                "20",
                "--no-pager",
                "-o",
                "cat"
            ]
        );
        assert!(log_argv("-x.service").is_err());
        assert!(log_argv("nope").is_err());
    }

    #[test]
    fn list_argv_asks_for_json_or_the_plain_table() {
        let json = list_units_argv(true);
        assert!(json.contains(&"--output=json".to_owned()));
        assert!(json.contains(&"--all".to_owned()) && json.contains(&"--user".to_owned()));
        let plain = list_units_argv(false);
        assert!(
            plain.contains(&"--plain".to_owned()) && !plain.contains(&"--output=json".to_owned())
        );
    }

    #[test]
    fn escapes_are_undone_for_display_and_bad_ones_survive() {
        assert_eq!(
            unescape_unit_name("app-gnome\\x2dkeyring\\x2dsecrets@autostart.service"),
            "app-gnome-keyring-secrets@autostart.service"
        );
        assert_eq!(unescape_unit_name("a\\xZZ.service"), "a\\xZZ.service");
        assert_eq!(unescape_unit_name("a\\x2"), "a\\x2");
        assert_eq!(unescape_unit_name("plain.service"), "plain.service");
        let unit = units()
            .into_iter()
            .find(|u| u.name.contains("keyring"))
            .unwrap();
        assert_eq!(unit.short_name(), "app-gnome-keyring-secrets@autostart");
    }

    #[test]
    fn default_order_puts_failed_then_active_then_the_rest() {
        let all = units();
        let names = order(&all)
            .into_iter()
            .map(|i| all[i].name.clone())
            .collect::<Vec<_>>();
        assert_eq!(names[0], "clipvault-history.service");
        assert_eq!(names[1], "app-dojo@autostart.service");
        assert_eq!(names[2], "pipewire.service");
        assert_eq!(
            names.last().unwrap(),
            "syncthing.service",
            "unloaded units sort last"
        );
    }

    #[test]
    fn search_ranks_exact_then_prefix_then_fuzzy_then_description() {
        let all = units();
        let found = |query: &str| {
            search(&all, query, &mut matcher())
                .into_iter()
                .map(|i| all[i].name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(found("").len(), all.len());
        assert_eq!(found("pipewire")[0], "pipewire.service");
        assert_eq!(found("pipewire.service")[0], "pipewire.service");
        assert_eq!(found("clip")[0], "clipvault-history.service");
        assert_eq!(
            found("keyring"),
            ["app-gnome\\x2dkeyring\\x2dsecrets@autostart.service"]
        );
        // Only a description contains it.
        assert_eq!(found("multimedia"), ["pipewire.service"]);
        assert!(found("zzzzz").is_empty());
        // `app` prefixes two units: the active one precedes the inactive one.
        let app = found("app");
        assert_eq!(app[0], "app-dojo@autostart.service");
    }

    #[test]
    fn log_parsing_cleans_control_characters_and_drops_notices() {
        let text = "first line\n\u{1b}[31mred\u{1b}[0m\tafter tab\n-- No entries --\n";
        assert_eq!(parse_log(text), ["first line", "[31mred[0m after tab"]);
        assert!(parse_log("").is_empty());
        let long = "x".repeat(900);
        let cleaned = clean_log_line(&long);
        assert_eq!(cleaned.chars().count(), LOG_LINE_CHARS + 1);
    }

    fn fake(
        calls: &RefCell<Vec<Vec<String>>>,
        replies: impl Fn(&[String]) -> Result<CommandOutput>,
    ) -> impl Fn(&[String], Duration) -> Result<CommandOutput> {
        move |argv, _timeout| {
            calls.borrow_mut().push(argv.to_vec());
            replies(argv)
        }
    }

    fn output(success: bool, stdout: &str, stderr: &str) -> Result<CommandOutput> {
        Ok(CommandOutput {
            success,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        })
    }

    #[test]
    fn list_uses_json_and_merges_unit_files() {
        let calls = RefCell::new(Vec::new());
        let runner = fake(&calls, |argv| {
            if argv.contains(&"list-unit-files".to_owned()) {
                output(true, FILES, "")
            } else {
                output(true, JSON, "")
            }
        });
        let listed = list_with(&runner).unwrap();
        assert_eq!(listed.len(), 6);
        assert_eq!(calls.borrow().len(), 2);
    }

    #[test]
    fn list_falls_back_to_the_plain_table_when_json_is_refused() {
        let calls = RefCell::new(Vec::new());
        let runner = fake(&calls, |argv| {
            if argv.contains(&"--output=json".to_owned()) {
                output(false, "", "unknown output mode")
            } else if argv.contains(&"--plain".to_owned()) {
                output(true, "a.service loaded active running A\n", "")
            } else {
                unreachable!("unexpected command {argv:?}")
            }
        });
        // The unit-file query also asks for JSON, so it fails and is ignored.
        let listed = list_with(&runner).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "a.service");
    }

    #[test]
    fn list_reports_an_unreachable_manager_and_survives_missing_unit_files() {
        let runner = |argv: &[String], _| -> Result<CommandOutput> {
            if argv.contains(&"--plain".to_owned()) || argv.contains(&"list-unit-files".to_owned())
            {
                output(false, "", "Failed to connect to bus: No medium found")
            } else {
                output(false, "", "Failed to connect to bus")
            }
        };
        let error = list_with(&runner).unwrap_err().to_string();
        assert!(error.contains("listing user services failed"), "{error}");
        assert!(error.contains("No medium found"));

        let json_only = |argv: &[String], _| -> Result<CommandOutput> {
            if argv.contains(&"list-unit-files".to_owned()) {
                output(false, "", "boom")
            } else {
                output(true, JSON, "")
            }
        };
        assert_eq!(list_with(&json_only).unwrap().len(), 5);
    }

    #[test]
    fn run_builds_argv_and_surfaces_stderr_on_failure() {
        let calls = RefCell::new(Vec::new());
        let runner = fake(&calls, |_| output(true, "", ""));
        run_with(&runner, Action::Start, "a.service").unwrap();
        assert_eq!(calls.borrow()[0][2], "start");

        let failing = |_: &[String], _| output(false, "", "Unit a.service not found.\n");
        let error = run_with(&failing, Action::Stop, "a.service")
            .unwrap_err()
            .to_string();
        assert_eq!(error, "stop a.service failed: Unit a.service not found.");

        // A bad name never reaches the runner.
        let never = |_: &[String], _| -> Result<CommandOutput> { panic!("must not run") };
        assert!(run_with(&never, Action::Stop, "--all.service").is_err());
        assert!(recent_log_with(&never, "x y.service").is_err());
    }

    #[test]
    fn recent_log_runs_journalctl_and_cleans_the_lines() {
        let calls = RefCell::new(Vec::new());
        let runner = fake(&calls, |_| output(true, "one\ntwo\n", ""));
        assert_eq!(
            recent_log_with(&runner, "a.service").unwrap(),
            ["one", "two"]
        );
        assert_eq!(calls.borrow()[0][0], "journalctl");
        let failing = |_: &[String], _| output(false, "", "No journal files were found.");
        assert!(recent_log_with(&failing, "a.service").is_err());
    }

    #[test]
    fn run_command_captures_output_and_enforces_the_deadline() {
        let echo = ["sh", "-c", "echo out; echo err >&2; exit 3"].map(str::to_owned);
        let result = run_command(&echo, Duration::from_secs(5)).unwrap();
        assert!(!result.success);
        assert_eq!(result.stdout.trim(), "out");
        assert_eq!(result.stderr.trim(), "err");

        let slow = ["sleep", "30"].map(str::to_owned);
        let started = Instant::now();
        let error = run_command(&slow, Duration::from_millis(100)).unwrap_err();
        assert!(error.to_string().contains("did not finish"));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(run_command(&["definitely-not-a-program".to_owned()], LIST_TIMEOUT).is_err());
        assert!(run_command(&[], LIST_TIMEOUT).is_err());
    }

    /// Read-only: lists the real user manager's services and reads one journal, or skips.
    #[test]
    fn live_smoke_lists_services_and_reads_a_journal() {
        let listed = match list() {
            Ok(listed) => listed,
            Err(error) => {
                eprintln!("skipped: no reachable user manager: {error:#}");
                return;
            }
        };
        assert!(!listed.is_empty());
        for unit in &listed {
            assert_eq!(validate_unit_name(&unit.name), Ok(()), "{}", unit.name);
        }
        let order = order(&listed);
        assert_eq!(order.len(), listed.len());
        let Some(active) = listed.iter().find(|unit| unit.is_active()) else {
            return;
        };
        match recent_log(&active.name) {
            Ok(lines) => assert!(lines.len() <= LOG_LINES, "{} lines", lines.len()),
            Err(error) => eprintln!("journal not readable here: {error:#}"),
        }
    }
}
