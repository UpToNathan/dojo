//! Look snapshots: the bounded copy of a user's palette, pre-blurred wallpaper,
//! display name, `[session]` settings and last greeting that the login screen
//! draws from, so it never reads a home directory (ADR 0003).
//!
//! Three seams, from the inside out.
//!
//! - [`Look`] is the typed value. Every piece is parsed and validated when it is
//!   built ([`Login`], [`Palette`], [`Wallpaper`], [`Line`]), so a `Look` that
//!   exists is safe to encode, store and paint. [`Look::encode`] and
//!   [`Look::decode`] are the pure translation to the five snapshot [`File`]s.
//! - [`Collector`] reads one user's files out of a home directory. It has no
//!   privilege logic of its own.
//! - [`store`] and [`load`] write and read `root/users/<login>/`.
//!
//! # Privilege contract for [`Collector`]
//!
//! The caller must already have dropped to the user (fork, `setgroups`,
//! `setgid`, `setuid`) before it opens the collector, and must hand the result
//! back to the privileged parent as plain bytes. Every open here then runs with
//! the user's own authority, so a hostile link or file can reach nothing the
//! user could not already read. What remains is denial of service, which is
//! bounded by reading only regular files, never blocking on a FIFO, and
//! stopping at each file's size cap.
//!
//! # Symlinks
//!
//! The theme store publishes `~/.local/state/dojo/themes/current` as a symlink
//! to an activation directory whose entries are symlinks into the generation
//! store, and `~/.config/dojo/config.toml` is routinely a Home Manager or
//! dotfile link. A collector that refuses every link therefore finds nothing on a
//! working desktop. [`Links::Follow`] resolves links the way the user's own
//! programs do; [`Links::Refuse`] rejects a link in any component of the path.
//! The greeter-side [`load`] always refuses links: the snapshot tree is written
//! by `dojo-dm` and contains none. Both modes also refuse `/proc` magic links.
//! The kernel must provide `openat2` (Linux 5.6).
//!
//! Home layout follows the XDG defaults. A user who moves `XDG_STATE_HOME` or
//! `XDG_CONFIG_HOME` is not seen, because the display manager has no user
//! environment to ask.

use std::fmt;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Cursor, Read as _, Write as _};
use std::os::fd::OwnedFd;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _, PermissionsExt as _};
use std::path::Path;

use rustix::fs::{Mode, OFlags, RenameFlags, ResolveFlags, openat2};

use super::host;
use super::settings::SessionSettings;
use super::{history, message};
use crate::config::{ThemeColors, parse_session_settings};

/// Largest `palette.json`.
pub const PALETTE_CAP: u64 = 64 * 1024;
/// Largest wallpaper PNG.
pub const WALLPAPER_CAP: u64 = 16 * 1024 * 1024;
/// Largest `session.toml`, and largest `config.toml` the collector will parse.
pub const SETTINGS_CAP: u64 = 16 * 1024;
/// Largest `identity` or greeting file.
pub const TEXT_CAP: u64 = 1024;
/// Widest and tallest accepted wallpaper. The theme renders 480 pixels wide, so
/// this leaves room while bounding what the greeter decodes.
pub const WALLPAPER_MAX_SIDE: u32 = 4096;

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// A user name that is safe as one path component and as a file name.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Login(String);

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidLogin;

impl fmt::Display for InvalidLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("login must be 1 to 32 of a-z, 0-9, _, - and ., and not start with . or -")
    }
}

impl std::error::Error for InvalidLogin {}

impl Login {
    pub fn parse(text: &str) -> Result<Login, InvalidLogin> {
        let allowed = |byte: u8| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' | b'.');
        let first_ok = !text.starts_with(['.', '-']);
        let valid = (1..=32).contains(&text.len()) && first_ok && text.bytes().all(allowed);
        valid.then(|| Login(text.to_owned())).ok_or(InvalidLogin)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One line of text of at most `TEXT_CAP - 1` bytes with no control characters
/// and single spaces, so the file is a line plus a newline inside its cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line(String);

impl Line {
    pub fn parse(text: &str) -> Option<Line> {
        if text.chars().any(char::is_control) {
            return None;
        }
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        (!flat.is_empty() && (flat.len() as u64) < TEXT_CAP).then_some(Line(flat))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The full name for the user chip: the first comma field of a GECOS entry.
pub fn full_name(gecos: &str) -> Option<Line> {
    Line::parse(gecos.split(',').next()?)
}

/// A `palette.json` that parsed as the theme's semantic palette and passed its
/// validation, with the colours the lock screen reads already extracted. The
/// validated bytes are kept so the snapshot holds exactly what was checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    bytes: Vec<u8>,
    colors: ThemeColors,
    error: u32,
}

impl Palette {
    pub fn parse(bytes: Vec<u8>) -> anyhow::Result<Palette> {
        use anyhow::Context as _;
        let semantic: crate::theme::SemanticPalette =
            serde_json::from_slice(&bytes).context("not a Dojo palette")?;
        semantic.validate()?;
        let roles = [
            "primary",
            "background",
            "foreground",
            "background_highlight",
            "border",
            "foreground_muted",
            "primary_foreground",
            "selection_foreground",
            "surface",
            "surface_raised",
        ];
        let object: serde_json::Map<_, _> = roles
            .into_iter()
            .map(|role| (role.to_owned(), serde_json::json!(semantic.color(role))))
            .collect();
        let colors =
            serde_json::from_value(serde_json::Value::Object(object)).context("palette colours")?;
        let error = std::str::from_utf8(&bytes)
            .ok()
            .and_then(host::error_colour)
            .context("palette has no error colour")?;
        Ok(Palette {
            bytes,
            colors,
            error,
        })
    }

    pub fn colors(&self) -> ThemeColors {
        self.colors
    }

    /// `colors.error` as `0xrrggbb`.
    pub fn error_color(&self) -> u32 {
        self.error
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// A PNG whose header decodes to a sane size. Only the header is read here; the
/// greeter decodes the pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wallpaper(Vec<u8>);

impl Wallpaper {
    pub fn parse(bytes: Vec<u8>) -> anyhow::Result<Wallpaper> {
        anyhow::ensure!(bytes.starts_with(PNG_SIGNATURE), "not a PNG");
        let (width, height) =
            image::ImageReader::with_format(Cursor::new(&bytes), image::ImageFormat::Png)
                .into_dimensions()?;
        anyhow::ensure!(
            (1..=WALLPAPER_MAX_SIDE).contains(&width) && (1..=WALLPAPER_MAX_SIDE).contains(&height),
            "wallpaper is {width}x{height}, larger than {WALLPAPER_MAX_SIDE}"
        );
        Ok(Wallpaper(bytes))
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// The five files of a snapshot. The table is the layout: name, size cap and
/// where the collector finds the original in a home directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum File {
    Palette,
    Wallpaper,
    Settings,
    Identity,
    Greeting,
}

impl File {
    pub const ALL: [File; 5] = [
        File::Palette,
        File::Wallpaper,
        File::Settings,
        File::Identity,
        File::Greeting,
    ];

    pub fn name(self) -> &'static str {
        match self {
            File::Palette => "palette.json",
            File::Wallpaper => crate::theme::lock_wallpaper::FILE_NAME,
            File::Settings => "session.toml",
            File::Identity => "identity",
            File::Greeting => "greeting-unlock",
        }
    }

    pub fn cap(self) -> u64 {
        match self {
            File::Palette => PALETTE_CAP,
            File::Wallpaper => WALLPAPER_CAP,
            File::Settings => SETTINGS_CAP,
            File::Identity | File::Greeting => TEXT_CAP,
        }
    }

    /// Where the original lives under a home directory. The identity comes from
    /// the account, not a file.
    fn home_path(self) -> Option<&'static str> {
        match self {
            File::Palette => Some(".local/state/dojo/themes/current/palette.json"),
            File::Wallpaper => Some(".local/state/dojo/themes/current/lock-wallpaper.png"),
            File::Settings => Some(".config/dojo/config.toml"),
            File::Identity => None,
            File::Greeting => Some(".local/state/dojo/session/unlock"),
        }
    }
}

/// Everything the login screen shows for one user. A piece is `None` when the
/// user has none or it was rejected; the greeter paints its defaults for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    pub login: Login,
    /// The GECOS full name. [`Look::display_name`] applies the first-word rule.
    pub full_name: Option<Line>,
    pub palette: Option<Palette>,
    pub wallpaper: Option<Wallpaper>,
    pub settings: Option<SessionSettings>,
    /// The last line shown on `Occasion::Unlock`, so selection never repeats it.
    pub last_greeting: Option<Line>,
}

impl Look {
    pub fn empty(login: Login) -> Look {
        Look {
            login,
            full_name: None,
            palette: None,
            wallpaper: None,
            settings: None,
            last_greeting: None,
        }
    }

    pub fn display_name(&self) -> String {
        message::display_name(
            self.full_name.as_ref().map(Line::as_str),
            self.login.as_str(),
        )
    }

    pub fn settings_or_default(&self) -> SessionSettings {
        self.settings.unwrap_or_default()
    }

    /// The bytes of each piece that is present, in the snapshot's layout.
    pub fn encode(&self) -> Vec<(File, Vec<u8>)> {
        let line = |line: &Line| history::format(line.as_str()).into_bytes();
        let pieces = [
            (
                File::Palette,
                self.palette.as_ref().map(|p| p.bytes.clone()),
            ),
            (
                File::Wallpaper,
                self.wallpaper.as_ref().map(|w| w.0.clone()),
            ),
            (
                File::Settings,
                self.settings.map(|s| render_settings(&s).into_bytes()),
            ),
            (File::Identity, self.full_name.as_ref().map(line)),
            (File::Greeting, self.last_greeting.as_ref().map(line)),
        ];
        pieces
            .into_iter()
            .filter_map(|(file, bytes)| Some((file, bytes?)))
            .collect()
    }

    /// Builds a look from what `read` returns for each file. A file that is
    /// absent leaves its piece empty; one that is unreadable or invalid does the
    /// same and is listed in `rejected`.
    pub fn decode(
        login: Login,
        mut read: impl FnMut(File) -> Result<Option<Vec<u8>>, ReadError>,
    ) -> Decoded {
        let mut rejected = Vec::new();
        let mut take = |file: File| -> Option<Vec<u8>> {
            match read(file) {
                Ok(bytes) => bytes,
                Err(error) => {
                    rejected.push(Problem {
                        file,
                        reason: error.to_string(),
                    });
                    None
                }
            }
        };
        let raw = File::ALL.map(&mut take);
        let [palette, wallpaper, settings, identity, greeting] = raw;

        let mut look = Look::empty(login);
        let mut reject = |file: File, error: anyhow::Error| {
            rejected.push(Problem {
                file,
                reason: format!("{error:#}"),
            });
        };
        look.palette = palette.and_then(|bytes| {
            Palette::parse(bytes)
                .map_err(|error| reject(File::Palette, error))
                .ok()
        });
        look.wallpaper = wallpaper.and_then(|bytes| {
            Wallpaper::parse(bytes)
                .map_err(|error| reject(File::Wallpaper, error))
                .ok()
        });
        look.settings = settings.and_then(|bytes| {
            parse_settings(&bytes)
                .map_err(|error| reject(File::Settings, error))
                .ok()
        });
        look.full_name = identity.and_then(|bytes| {
            parse_line(&bytes)
                .map_err(|error| reject(File::Identity, error))
                .ok()
        });
        look.last_greeting = greeting.and_then(|bytes| {
            parse_line(&bytes)
                .map_err(|error| reject(File::Greeting, error))
                .ok()
        });
        Decoded { look, rejected }
    }
}

fn render_settings(settings: &SessionSettings) -> String {
    let SessionSettings {
        transition,
        greeting,
        farewell,
        reduced_motion,
    } = *settings;
    format!(
        "[session]\ntransition = {transition}\ngreeting = {greeting}\nfarewell = {farewell}\nreduced_motion = {reduced_motion}\n"
    )
}

/// Parses `session.toml` and, with the same rules, the user's `config.toml`.
fn parse_settings(bytes: &[u8]) -> anyhow::Result<SessionSettings> {
    parse_session_settings(std::str::from_utf8(bytes)?)
}

fn parse_line(bytes: &[u8]) -> anyhow::Result<Line> {
    history::parse(std::str::from_utf8(bytes)?)
        .and_then(|line| Line::parse(&line))
        .ok_or_else(|| anyhow::anyhow!("not a single line of text"))
}

/// A file that existed but was not accepted, and why.
#[derive(Debug, PartialEq, Eq)]
pub struct Problem {
    pub file: File,
    pub reason: String,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.file.name(), self.reason)
    }
}

#[derive(Debug)]
pub struct Decoded {
    pub look: Look,
    pub rejected: Vec<Problem>,
}

impl Decoded {
    /// For each rejected file, keeps what the previous snapshot had instead of
    /// dropping it. A file that is simply absent is not rejected: the user has
    /// no such piece now, and the old one may no longer match the new palette.
    pub fn merged_over(self, previous: Option<&Look>) -> Look {
        let Decoded { mut look, rejected } = self;
        let Some(previous) = previous else {
            return look;
        };
        for Problem { file, .. } in rejected {
            match file {
                File::Palette => look.palette = previous.palette.clone(),
                File::Wallpaper => look.wallpaper = previous.wallpaper.clone(),
                File::Settings => look.settings = previous.settings,
                File::Identity => look.full_name = previous.full_name.clone(),
                File::Greeting => look.last_greeting = previous.last_greeting.clone(),
            }
        }
        look
    }
}

/// How the collector treats symbolic links on the way to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Links {
    /// Resolve them like the user's own programs. See the module notes.
    Follow,
    /// Reject a link in any component, including swapped-in directories.
    Refuse,
}

#[derive(Debug)]
pub enum ReadError {
    TooLarge(u64),
    NotRegular,
    Symlink,
    Io(io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::TooLarge(cap) => write!(f, "larger than {cap} bytes"),
            ReadError::NotRegular => f.write_str("not a regular file"),
            ReadError::Symlink => f.write_str("a symbolic link is in the way"),
            ReadError::Io(error) => write!(f, "{error}"),
        }
    }
}

/// An open directory that paths are resolved beneath with `openat2`, so a
/// component swapped for a link between two reads cannot redirect them.
struct Tree {
    directory: OwnedFd,
    links: Links,
}

impl Tree {
    fn open(path: &Path, links: Links) -> io::Result<Tree> {
        let directory = rustix::fs::open(
            path,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        Ok(Tree { directory, links })
    }

    fn resolve(&self) -> ResolveFlags {
        match self.links {
            Links::Follow => ResolveFlags::NO_MAGICLINKS,
            Links::Refuse => ResolveFlags::NO_MAGICLINKS | ResolveFlags::NO_SYMLINKS,
        }
    }

    fn subdirectory(&self, relative: &str) -> io::Result<Tree> {
        let directory = openat2(
            &self.directory,
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
            self.resolve(),
        )?;
        Ok(Tree {
            directory,
            links: self.links,
        })
    }

    /// `Ok(None)` is a file that is not there. Never blocks on a FIFO.
    fn read(&self, relative: &str, cap: u64) -> Result<Option<Vec<u8>>, ReadError> {
        let opened = openat2(
            &self.directory,
            relative,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY,
            Mode::empty(),
            self.resolve(),
        );
        let fd = match opened {
            Ok(fd) => fd,
            Err(rustix::io::Errno::NOENT) => return Ok(None),
            Err(rustix::io::Errno::LOOP) if self.links == Links::Refuse => {
                return Err(ReadError::Symlink);
            }
            Err(error) => return Err(ReadError::Io(error.into())),
        };
        let file = std::fs::File::from(fd);
        let metadata = file.metadata().map_err(ReadError::Io)?;
        if !metadata.is_file() {
            return Err(ReadError::NotRegular);
        }
        if metadata.len() > cap {
            return Err(ReadError::TooLarge(cap));
        }
        let mut bytes = Vec::new();
        file.take(cap + 1)
            .read_to_end(&mut bytes)
            .map_err(ReadError::Io)?;
        if bytes.len() as u64 > cap {
            return Err(ReadError::TooLarge(cap));
        }
        Ok(Some(bytes))
    }
}

/// Reads one user's look out of their home directory. See the module notes for
/// the privilege contract: open it only after dropping to that user.
pub struct Collector {
    home: Tree,
}

impl Collector {
    /// `home` itself is opened as given, since it comes from the account
    /// database; `links` applies to everything beneath it.
    pub fn open(home: &Path, links: Links) -> io::Result<Collector> {
        Ok(Collector {
            home: Tree::open(home, links)?,
        })
    }

    /// Each piece is collected on its own. A missing `config.toml` means the
    /// default settings, as it does for the shell and the locker.
    pub fn collect(&self, login: Login, gecos: Option<&str>) -> Decoded {
        let mut decoded = Look::decode(login, |file| {
            let Some(path) = file.home_path() else {
                return Ok(None);
            };
            let bytes = self.home.read(path, file.cap())?;
            Ok(match (file, bytes) {
                (File::Settings, None) => Some(Vec::new()),
                (_, bytes) => bytes,
            })
        });
        decoded.look.full_name = gecos.and_then(full_name);
        decoded
    }
}

fn directory_mode(path: &Path) -> io::Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o750))
}

fn write_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o640)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o640))?;
    file.write_all(bytes)?;
    file.sync_all()
}

fn sync_directory(path: &Path) -> io::Result<()> {
    fs::File::open(path)?.sync_all()
}

/// Writes `root/users/<login>/` as a unit: stage in a sibling directory, then
/// swap it into place. The previous snapshot stays whole until the swap and is
/// untouched if anything before it fails. Files are 0640 and directories 0750
/// whatever the umask; the group is the installer's concern.
///
/// There must be one writer per login at a time.
pub fn store(root: &Path, look: &Look) -> io::Result<()> {
    let users = root.join("users");
    if !users.is_dir() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o750)
            .create(&users)?;
        directory_mode(&users)?;
    }
    let login = look.login.as_str();
    // `~` cannot occur in a login, so this prefix never matches another user's.
    let prefix = format!(".tmp-{login}~");
    sweep(&users, &prefix);

    let staging = tempfile::Builder::new()
        .prefix(&prefix)
        .tempdir_in(&users)?;
    directory_mode(staging.path())?;
    for (file, bytes) in look.encode() {
        write_file(&staging.path().join(file.name()), &bytes)?;
    }
    sync_directory(staging.path())?;

    let target = users.join(login);
    match rustix::fs::renameat_with(
        rustix::fs::CWD,
        staging.path(),
        rustix::fs::CWD,
        &target,
        RenameFlags::EXCHANGE,
    ) {
        Ok(()) => {}
        Err(rustix::io::Errno::NOENT) => fs::rename(staging.path(), &target)?,
        Err(error) => return Err(error.into()),
    }
    // After an exchange the staging path holds the old snapshot; dropping the
    // guard removes it. After a rename it is gone and the removal is a no-op.
    drop(staging);
    sync_directory(&users)
}

/// Removes staging directories a crashed writer left for this login.
fn sweep(users: &Path, prefix: &str) {
    let Ok(entries) = fs::read_dir(users) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name().to_string_lossy().starts_with(prefix) {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// The greeter's reader. Touches only `root/users/<login>/`, follows no link,
/// and returns `None` for an invalid login, a missing snapshot or one with any
/// piece present but invalid. A piece that is simply absent is a partial
/// snapshot and loads.
pub fn load(root: &Path, login: &str) -> Option<Look> {
    let login = Login::parse(login).ok()?;
    let tree = Tree::open(root, Links::Refuse)
        .and_then(|root| root.subdirectory(&format!("users/{}", login.as_str())))
        .ok()?;
    let decoded = Look::decode(login, |file| tree.read(file.name(), file.cap()));
    decoded.rejected.is_empty().then_some(decoded.look)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;

    fn palette_bytes() -> Vec<u8> {
        let preset = &crate::theme::presets().unwrap()[0];
        serde_json::to_vec_pretty(&preset.palette).unwrap()
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(width, height)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn login(text: &str) -> Login {
        Login::parse(text).unwrap()
    }

    fn full_look() -> Look {
        Look {
            login: login("alyssa"),
            full_name: Line::parse("Alyssa Park"),
            palette: Some(Palette::parse(palette_bytes()).unwrap()),
            wallpaper: Some(Wallpaper::parse(png_bytes(48, 27)).unwrap()),
            settings: Some(SessionSettings {
                transition: false,
                greeting: true,
                farewell: false,
                reduced_motion: true,
            }),
            last_greeting: Line::parse("Welcome back, Alyssa"),
        }
    }

    /// A home directory holding every original a collector reads.
    fn home(dir: &Path) {
        let theme = dir.join(".local/state/dojo/themes/current");
        let session = dir.join(".local/state/dojo/session");
        let config = dir.join(".config/dojo");
        for directory in [&theme, &session, &config] {
            fs::create_dir_all(directory).unwrap();
        }
        fs::write(theme.join("palette.json"), palette_bytes()).unwrap();
        fs::write(theme.join("lock-wallpaper.png"), png_bytes(48, 27)).unwrap();
        fs::write(session.join("unlock"), "Hajime, Alyssa\n").unwrap();
        fs::write(config.join("config.toml"), "[session]\ngreeting = false\n").unwrap();
    }

    fn collect(dir: &Path, links: Links) -> Decoded {
        Collector::open(dir, links)
            .unwrap()
            .collect(login("alyssa"), Some("Alyssa Park,,,"))
    }

    fn rejected_files(decoded: &Decoded) -> Vec<File> {
        decoded
            .rejected
            .iter()
            .map(|problem| problem.file)
            .collect()
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn a_snapshot_round_trips_with_group_readable_modes() {
        let root = tempfile::tempdir().unwrap();
        let look = full_look();
        store(root.path(), &look).unwrap();

        assert_eq!(load(root.path(), "alyssa"), Some(look.clone()));
        let directory = root.path().join("users/alyssa");
        let mut names: Vec<_> = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "greeting-unlock",
                "identity",
                "lock-wallpaper.png",
                "palette.json",
                "session.toml"
            ]
        );
        assert_eq!(mode(&directory), 0o750);
        assert_eq!(mode(&root.path().join("users")), 0o750);
        for name in names {
            assert_eq!(mode(&directory.join(name)), 0o640);
        }
        assert_eq!(look.display_name(), "Alyssa");
    }

    #[test]
    fn the_palette_gives_the_lock_screen_its_colours_and_error_red() {
        let preset = &crate::theme::presets().unwrap()[0].palette;
        let palette = Palette::parse(palette_bytes()).unwrap();
        assert_eq!(
            palette.colors().background.hex(),
            preset.color("background")
        );
        assert_eq!(palette.colors().primary.hex(), preset.color("primary"));
        assert_eq!(
            format!("#{:06x}", palette.error_color()),
            preset.color("error")
        );
    }

    #[test]
    fn a_collector_reads_every_piece_of_a_home() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let decoded = collect(dir.path(), Links::Refuse);
        assert!(decoded.rejected.is_empty(), "{:?}", decoded.rejected);
        let look = decoded.look;
        assert_eq!(look.full_name, Line::parse("Alyssa Park"));
        assert_eq!(look.last_greeting, Line::parse("Hajime, Alyssa"));
        assert_eq!(
            look.settings,
            Some(SessionSettings {
                greeting: false,
                ..Default::default()
            })
        );
        assert!(look.palette.is_some() && look.wallpaper.is_some());
    }

    #[test]
    fn home_paths_agree_with_the_history_and_theme_layout() {
        let relative = |path: Option<&str>| path.unwrap().rsplit_once('/').unwrap().0.to_owned();
        let sessions = history::directory_from(None, Some("/h".into())).unwrap();
        assert_eq!(
            Path::new("/h").join(relative(File::Greeting.home_path())),
            sessions
        );
        assert_eq!(
            File::Greeting.home_path().unwrap().rsplit('/').next(),
            Some(crate::session::Occasion::Unlock.key())
        );
        assert!(
            File::Greeting
                .name()
                .ends_with(crate::session::Occasion::Unlock.key())
        );
        let theme = crate::theme::Paths {
            home: "/h".into(),
            config: "/h/.config".into(),
            data: "/h/.local/share".into(),
            root: "/h/.local/state/dojo/themes".into(),
        };
        for file in [File::Palette, File::Wallpaper] {
            assert_eq!(
                theme.root.join("current").join(file.name()),
                Path::new("/h").join(file.home_path().unwrap())
            );
        }
        assert_eq!(
            Path::new("/h").join(File::Settings.home_path().unwrap()),
            theme.config.join("dojo/config.toml")
        );
    }

    #[test]
    fn a_missing_wallpaper_and_config_still_give_a_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        fs::remove_file(
            dir.path()
                .join(".local/state/dojo/themes/current/lock-wallpaper.png"),
        )
        .unwrap();
        fs::remove_file(dir.path().join(".config/dojo/config.toml")).unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert!(decoded.rejected.is_empty());
        assert_eq!(decoded.look.wallpaper, None);
        assert_eq!(decoded.look.settings, Some(SessionSettings::default()));
        assert!(decoded.look.palette.is_some());

        let root = tempfile::tempdir().unwrap();
        store(root.path(), &decoded.look).unwrap();
        assert_eq!(load(root.path(), "alyssa"), Some(decoded.look));
    }

    #[test]
    fn an_empty_home_gives_an_empty_look_with_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert!(decoded.rejected.is_empty());
        assert_eq!(decoded.look.palette, None);
        assert_eq!(
            decoded.look.settings_or_default(),
            SessionSettings::default()
        );
        assert_eq!(decoded.look.display_name(), "Alyssa");
    }

    #[test]
    fn the_display_name_follows_the_first_word_rule() {
        let mut look = Look::empty(login("apark"));
        assert_eq!(look.display_name(), "Apark");
        look.full_name = full_name("Grace Hopper,Room 4");
        assert_eq!(look.display_name(), "Grace");
        assert_eq!(full_name(",,,"), None);
    }

    fn oversized(dir: &Path, file: File, path: &str, filler: u8) {
        let target = dir.join(path);
        let mut bytes = match file {
            File::Palette => palette_bytes(),
            File::Wallpaper => png_bytes(8, 8),
            File::Settings => b"[session]\n".to_vec(),
            File::Identity | File::Greeting => b"x".to_vec(),
        };
        bytes.resize(file.cap() as usize + 1, filler);
        fs::write(target, bytes).unwrap();
    }

    #[test]
    fn each_cap_rejects_the_file_at_one_byte_over() {
        for (file, filler) in [
            (File::Palette, b' '),
            (File::Wallpaper, 0),
            (File::Settings, b'\n'),
            (File::Greeting, b'x'),
        ] {
            let dir = tempfile::tempdir().unwrap();
            home(dir.path());
            oversized(dir.path(), file, file.home_path().unwrap(), filler);
            let decoded = collect(dir.path(), Links::Refuse);
            assert_eq!(rejected_files(&decoded), [file], "{file:?}");
            assert!(
                decoded.rejected[0].reason.contains("larger than"),
                "{file:?}: {}",
                decoded.rejected[0].reason
            );
        }
    }

    #[test]
    fn a_file_exactly_at_the_cap_is_read() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let mut palette = palette_bytes();
        palette.resize(PALETTE_CAP as usize, b' ');
        fs::write(dir.path().join(File::Palette.home_path().unwrap()), palette).unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert!(decoded.rejected.is_empty());
        assert!(decoded.look.palette.is_some());
    }

    #[test]
    fn the_identity_and_greeting_text_has_the_text_cap() {
        let long = "x".repeat(TEXT_CAP as usize);
        assert_eq!(Line::parse(&long), None);
        assert!(Line::parse(&"x".repeat(TEXT_CAP as usize - 1)).is_some());
        assert_eq!(full_name(&long), None);
        assert_eq!(Line::parse("bell\u{7}"), None);
        assert_eq!(Line::parse("two\nlines"), None);
        assert_eq!(Line::parse("  a   b "), Line::parse("a b"));

        let root = tempfile::tempdir().unwrap();
        store(root.path(), &full_look()).unwrap();
        let identity = root.path().join("users/alyssa/identity");
        fs::write(&identity, vec![b'x'; TEXT_CAP as usize + 1]).unwrap();
        assert_eq!(load(root.path(), "alyssa"), None);
    }

    #[test]
    fn a_malformed_palette_is_skipped_and_the_rest_survives() {
        let valid: serde_json::Value = serde_json::from_slice(&palette_bytes()).unwrap();
        let mut missing_role = valid.clone();
        missing_role["colors"]
            .as_object_mut()
            .unwrap()
            .remove("error");
        let mut bad_hex = valid.clone();
        bad_hex["colors"]["primary"] = "blue".into();
        let mut extra = valid;
        extra["surprise"] = 1.into();
        for bytes in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&missing_role).unwrap(),
            serde_json::to_vec(&bad_hex).unwrap(),
            serde_json::to_vec(&extra).unwrap(),
            vec![0xff, 0xfe],
        ] {
            let dir = tempfile::tempdir().unwrap();
            home(dir.path());
            fs::write(dir.path().join(File::Palette.home_path().unwrap()), bytes).unwrap();
            let decoded = collect(dir.path(), Links::Refuse);
            assert_eq!(rejected_files(&decoded), [File::Palette]);
            assert_eq!(decoded.look.palette, None);
            assert!(decoded.look.wallpaper.is_some());
            assert_eq!(decoded.look.last_greeting, Line::parse("Hajime, Alyssa"));
        }
    }

    #[test]
    fn wallpapers_must_be_pngs_of_a_sane_size() {
        assert!(Wallpaper::parse(png_bytes(480, 270)).is_ok());
        assert!(Wallpaper::parse(b"GIF89a".to_vec()).is_err());
        assert!(Wallpaper::parse(PNG_SIGNATURE.to_vec()).is_err());
        let truncated = png_bytes(16, 16)[..20].to_vec();
        assert!(Wallpaper::parse(truncated).is_err());
        assert!(Wallpaper::parse(png_bytes(WALLPAPER_MAX_SIDE + 1, 1)).is_err());
        let mut jpeg = Vec::new();
        image::RgbImage::new(8, 8)
            .write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
            .unwrap();
        assert!(Wallpaper::parse(jpeg).is_err());
    }

    #[test]
    fn invalid_settings_are_rejected_not_defaulted() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let config = dir.path().join(File::Settings.home_path().unwrap());
        for invalid in [
            "[session]\ngreting = false",
            "[session]\ngreeting = 'no'",
            "\u{0}",
        ] {
            fs::write(&config, invalid).unwrap();
            let decoded = collect(dir.path(), Links::Refuse);
            assert_eq!(rejected_files(&decoded), [File::Settings], "{invalid}");
            assert_eq!(decoded.look.settings, None);
        }
    }

    #[test]
    fn refuse_rejects_a_linked_file_and_a_linked_directory() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let outside = tempfile::tempdir().unwrap();
        let palette = dir.path().join(File::Palette.home_path().unwrap());
        fs::rename(&palette, outside.path().join("palette.json")).unwrap();
        symlink(outside.path().join("palette.json"), &palette).unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert_eq!(rejected_files(&decoded), [File::Palette]);
        assert!(decoded.rejected[0].reason.contains("symbolic link"));

        let themes = dir.path().join(".local/state/dojo/themes");
        fs::rename(themes.join("current"), outside.path().join("current")).unwrap();
        symlink(outside.path().join("current"), themes.join("current")).unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert_eq!(rejected_files(&decoded), [File::Palette, File::Wallpaper]);
        assert!(decoded.look.palette.is_none() && decoded.look.wallpaper.is_none());
        assert!(decoded.look.last_greeting.is_some());
    }

    #[test]
    fn refuse_rejects_a_home_subdirectory_swapped_for_a_link() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let elsewhere = tempfile::tempdir().unwrap();
        fs::write(
            elsewhere.path().join("config.toml"),
            "[session]\nfarewell = false",
        )
        .unwrap();
        fs::remove_dir_all(dir.path().join(".config/dojo")).unwrap();
        symlink(elsewhere.path(), dir.path().join(".config/dojo")).unwrap();
        let decoded = collect(dir.path(), Links::Refuse);
        assert_eq!(rejected_files(&decoded), [File::Settings]);
    }

    #[test]
    fn follow_resolves_the_theme_stores_own_link_layout() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join(".local/state/dojo");
        let generation = state.join("themes/generations/g/dark");
        let activation = state.join("themes/activations/a");
        fs::create_dir_all(&generation).unwrap();
        fs::create_dir_all(&activation).unwrap();
        fs::write(generation.join("palette.json"), palette_bytes()).unwrap();
        fs::write(generation.join("lock-wallpaper.png"), png_bytes(8, 8)).unwrap();
        for name in ["palette.json", "lock-wallpaper.png"] {
            symlink(generation.join(name), activation.join(name)).unwrap();
        }
        symlink("activations/a", state.join("themes/current")).unwrap();

        let followed = collect(dir.path(), Links::Follow);
        assert!(followed.rejected.is_empty(), "{:?}", followed.rejected);
        assert!(followed.look.palette.is_some() && followed.look.wallpaper.is_some());

        let refused = collect(dir.path(), Links::Refuse);
        assert_eq!(rejected_files(&refused), [File::Palette, File::Wallpaper]);
    }

    #[test]
    fn a_loop_or_a_special_file_is_rejected_without_hanging() {
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        let greeting = dir.path().join(File::Greeting.home_path().unwrap());
        fs::remove_file(&greeting).unwrap();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            &greeting,
            rustix::fs::FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let palette = dir.path().join(File::Palette.home_path().unwrap());
        fs::remove_file(&palette).unwrap();
        fs::create_dir(&palette).unwrap();
        let wallpaper = dir.path().join(File::Wallpaper.home_path().unwrap());
        fs::remove_file(&wallpaper).unwrap();
        symlink(&wallpaper, &wallpaper).unwrap();

        let decoded = collect(dir.path(), Links::Follow);
        assert_eq!(
            rejected_files(&decoded),
            [File::Palette, File::Wallpaper, File::Greeting]
        );
        assert!(decoded.rejected[0].reason.contains("not a regular file"));
    }

    #[test]
    fn a_rejected_piece_keeps_the_previous_snapshot_and_an_absent_one_clears() {
        let previous = full_look();
        let dir = tempfile::tempdir().unwrap();
        home(dir.path());
        fs::write(dir.path().join(File::Palette.home_path().unwrap()), "junk").unwrap();
        fs::remove_file(dir.path().join(File::Wallpaper.home_path().unwrap())).unwrap();

        let merged = collect(dir.path(), Links::Refuse).merged_over(Some(&previous));
        assert_eq!(merged.palette, previous.palette);
        assert_eq!(merged.wallpaper, None);
        assert_eq!(merged.last_greeting, Line::parse("Hajime, Alyssa"));

        let first_login = collect(dir.path(), Links::Refuse).merged_over(None);
        assert_eq!(first_login.palette, None);
    }

    #[test]
    fn a_partial_snapshot_loads_and_a_corrupt_one_is_ignored() {
        let root = tempfile::tempdir().unwrap();
        let mut look = Look::empty(login("alyssa"));
        look.palette = full_look().palette;
        store(root.path(), &look).unwrap();
        assert_eq!(load(root.path(), "alyssa"), Some(look));

        let palette = root.path().join("users/alyssa/palette.json");
        fs::write(&palette, "{}").unwrap();
        assert_eq!(load(root.path(), "alyssa"), None);
        assert_eq!(load(root.path(), "nobody"), None);
        assert_eq!(load(&root.path().join("missing"), "alyssa"), None);
    }

    #[test]
    fn load_refuses_links_in_the_snapshot_tree() {
        let root = tempfile::tempdir().unwrap();
        store(root.path(), &full_look()).unwrap();
        let elsewhere = tempfile::tempdir().unwrap();

        let file = root.path().join("users/alyssa/identity");
        fs::rename(&file, elsewhere.path().join("identity")).unwrap();
        symlink(elsewhere.path().join("identity"), &file).unwrap();
        assert_eq!(load(root.path(), "alyssa"), None);

        let other = tempfile::tempdir().unwrap();
        store(other.path(), &full_look()).unwrap();
        let directory = other.path().join("users/alyssa");
        fs::rename(&directory, elsewhere.path().join("alyssa")).unwrap();
        symlink(elsewhere.path().join("alyssa"), &directory).unwrap();
        assert_eq!(load(other.path(), "alyssa"), None);

        let users_link = tempfile::tempdir().unwrap();
        symlink(elsewhere.path(), users_link.path().join("users")).unwrap();
        assert_eq!(load(users_link.path(), "alyssa"), None);
    }

    #[test]
    fn store_replaces_the_previous_snapshot_whole() {
        let root = tempfile::tempdir().unwrap();
        store(root.path(), &full_look()).unwrap();
        let mut second = Look::empty(login("alyssa"));
        second.last_greeting = Line::parse("Hajime, Alyssa");
        store(root.path(), &second).unwrap();

        assert_eq!(load(root.path(), "alyssa"), Some(second));
        let names: Vec<_> = fs::read_dir(root.path().join("users"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names, ["alyssa"], "no staging directory left behind");
        let files = fs::read_dir(root.path().join("users/alyssa"))
            .unwrap()
            .count();
        assert_eq!(files, 1, "nothing of the old snapshot survives the swap");
    }

    #[test]
    fn a_failed_store_leaves_the_previous_snapshot_untouched() {
        if rustix::process::geteuid().is_root() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let before = full_look();
        store(root.path(), &before).unwrap();

        let users = root.path().join("users");
        fs::set_permissions(&users, fs::Permissions::from_mode(0o550)).unwrap();
        let mut after = before.clone();
        after.last_greeting = Line::parse("Different");
        let result = store(root.path(), &after);
        fs::set_permissions(&users, fs::Permissions::from_mode(0o750)).unwrap();

        assert!(result.is_err());
        assert_eq!(load(root.path(), "alyssa"), Some(before));
        assert_eq!(fs::read_dir(&users).unwrap().count(), 1);
    }

    #[test]
    fn a_crashed_writers_staging_directory_is_swept_for_that_login_only() {
        let root = tempfile::tempdir().unwrap();
        store(root.path(), &full_look()).unwrap();
        let users = root.path().join("users");
        fs::create_dir(users.join(".tmp-alyssa~stale")).unwrap();
        fs::create_dir(users.join(".tmp-alyssa-b~theirs")).unwrap();
        store(root.path(), &full_look()).unwrap();
        assert!(!users.join(".tmp-alyssa~stale").exists());
        assert!(users.join(".tmp-alyssa-b~theirs").exists());
    }

    #[test]
    fn a_store_into_an_empty_root_creates_the_layout() {
        let root = tempfile::tempdir().unwrap();
        let nested = root.path().join("var/lib/dojo-dm");
        fs::create_dir_all(&nested).unwrap();
        store(&nested, &Look::empty(login("n"))).unwrap();
        assert_eq!(load(&nested, "n"), Some(Look::empty(login("n"))));
    }

    #[test]
    fn logins_are_one_safe_path_component() {
        for valid in [
            "a",
            "alyssa",
            "a.b-c_d9",
            "0day",
            &"a".repeat(32),
            "a.",
            "a-",
        ] {
            assert!(Login::parse(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "",
            ".",
            "..",
            ".hidden",
            "-rf",
            "../etc",
            "a/b",
            "/abs",
            "a\\b",
            "UPPER",
            "Alyssa",
            "a b",
            "a\0b",
            "a\n",
            "é",
            "a~b",
            &"a".repeat(33),
        ] {
            assert_eq!(Login::parse(invalid), Err(InvalidLogin), "{invalid:?}");
        }
    }

    #[test]
    fn a_traversing_login_never_reaches_outside_the_snapshot_root() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("snapshots");
        fs::create_dir(&root).unwrap();
        store(&root, &full_look()).unwrap();
        store(base.path(), &Look::empty(login("secret"))).unwrap();

        assert!(load(&root, "alyssa").is_some());
        for attempt in [
            "../users/secret",
            "../../users/secret",
            "alyssa/../alyssa",
            "./alyssa",
            "alyssa/",
            "/users/secret",
        ] {
            assert_eq!(load(&root, attempt), None, "{attempt}");
        }
    }
}
