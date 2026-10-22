//! What the login screen remembers between boots: the last user and the last session,
//! two tiny files in the service's state directory. The service writes them with a
//! rename, so a reader sees a whole file or none, and trusts nothing it reads: a file
//! that is a link, not its own, writable by others or not exactly a valid value is
//! treated as absent.

use std::fs::{self, File};
use std::io::{self, Read as _, Write as _};
use std::os::unix::fs::{MetadataExt as _, OpenOptionsExt as _};
use std::path::Path;

use super::protocol::{Login, SessionId};

pub const LAST_USER: &str = "last-user";
pub const LAST_SESSION: &str = "last-session";
/// A login or a session id plus its newline fits well inside this.
const MAX_BYTES: u64 = 128;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Remembered {
    pub login: Option<Login>,
    pub session: Option<SessionId>,
}

fn value(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    text.strip_suffix('\n').or(Some(text))
}

fn decode_login(bytes: &[u8]) -> Option<Login> {
    Login::parse(value(bytes)?).ok()
}

fn decode_session(bytes: &[u8]) -> Option<SessionId> {
    SessionId::parse(value(bytes)?)
}

/// The file's bytes if it is a small regular file that only this process could have written.
fn read_own_file(path: &Path) -> Option<Vec<u8>> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_NOCTTY | libc::O_CLOEXEC)
        .open(path)
        .ok()?;
    let metadata = file.metadata().ok()?;
    let own = metadata.is_file()
        && metadata.uid() == unsafe { libc::geteuid() }
        && metadata.mode() & 0o022 == 0;
    if !own {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= MAX_BYTES).then_some(bytes)
}

pub fn load(directory: &Path) -> Remembered {
    Remembered {
        login: read_own_file(&directory.join(LAST_USER)).and_then(|bytes| decode_login(&bytes)),
        session: read_own_file(&directory.join(LAST_SESSION))
            .and_then(|bytes| decode_session(&bytes)),
    }
}

fn save(directory: &Path, name: &str, text: &str) -> io::Result<()> {
    let target = directory.join(name);
    if read_own_file(&target).as_deref().and_then(value) == Some(text) {
        return Ok(());
    }
    let mut staged = tempfile::Builder::new()
        .prefix(".tmp-")
        .tempfile_in(directory)?;
    staged.write_all(text.as_bytes())?;
    staged.write_all(b"\n")?;
    staged.as_file().sync_all()?;
    staged.persist(&target).map_err(|error| error.error)?;
    File::open(directory)?.sync_all()
}

pub fn save_login(directory: &Path, login: &Login) -> io::Result<()> {
    save(directory, LAST_USER, login.as_str())
}

pub fn save_session(directory: &Path, session: &SessionId) -> io::Result<()> {
    save(directory, LAST_SESSION, session.as_str())
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    use super::*;

    fn login(text: &str) -> Login {
        Login::parse(text).unwrap()
    }

    fn session(text: &str) -> SessionId {
        SessionId::parse(text).unwrap()
    }

    #[test]
    fn values_are_one_token_with_an_optional_newline() {
        assert_eq!(decode_login(b"nathan\n"), Some(login("nathan")));
        assert_eq!(decode_login(b"nathan"), Some(login("nathan")));
        for bad in [
            &b""[..],
            b"\n",
            b"Nathan",
            b"nathan\n\n",
            b"nathan\nroot\n",
            b" nathan",
            b"nathan\0",
            b"../x",
            b"\xff",
            &[b'a'; 33],
        ] {
            assert_eq!(decode_login(bad), None, "{bad:?}");
        }
        assert_eq!(decode_session(b"niri-uwsm\n"), Some(session("niri-uwsm")));
        assert_eq!(decode_session(b"../etc/passwd"), None);
        assert_eq!(decode_session(b"a b"), None);
    }

    #[test]
    fn what_was_saved_is_loaded_and_nothing_is_left_behind() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load(dir.path()), Remembered::default());
        save_login(dir.path(), &login("alyssa")).unwrap();
        save_session(dir.path(), &session("niri-uwsm")).unwrap();
        assert_eq!(
            load(dir.path()),
            Remembered {
                login: Some(login("alyssa")),
                session: Some(session("niri-uwsm"))
            }
        );
        save_login(dir.path(), &login("nathan")).unwrap();
        save_login(dir.path(), &login("nathan")).unwrap();
        assert_eq!(load(dir.path()).login, Some(login("nathan")));
        let mut names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        assert_eq!(names, [LAST_SESSION, LAST_USER]);
        assert_eq!(fs::read(dir.path().join(LAST_USER)).unwrap(), b"nathan\n");
    }

    #[test]
    fn files_that_are_links_loose_oversized_or_invalid_read_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = |name: &str| dir.path().join(name);
        fs::write(path("real"), "alyssa\n").unwrap();
        symlink(path("real"), path(LAST_USER)).unwrap();
        assert_eq!(load(dir.path()).login, None, "a link");

        fs::remove_file(path(LAST_USER)).unwrap();
        fs::write(path(LAST_USER), "alyssa\n").unwrap();
        fs::set_permissions(path(LAST_USER), fs::Permissions::from_mode(0o666)).unwrap();
        assert_eq!(load(dir.path()).login, None, "writable by others");

        fs::set_permissions(path(LAST_USER), fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(load(dir.path()).login, Some(login("alyssa")));
        fs::write(path(LAST_USER), "x".repeat(129)).unwrap();
        assert_eq!(load(dir.path()).login, None, "too large");

        fs::remove_file(path(LAST_USER)).unwrap();
        fs::create_dir(path(LAST_USER)).unwrap();
        assert_eq!(load(dir.path()).login, None, "a directory");
    }

    #[test]
    fn saving_over_a_link_replaces_the_link_not_its_target() {
        let dir = tempfile::tempdir().unwrap();
        let outside = dir.path().join("outside");
        fs::write(&outside, "keep\n").unwrap();
        let state = dir.path().join("state");
        fs::create_dir(&state).unwrap();
        symlink(&outside, state.join(LAST_USER)).unwrap();
        save_login(&state, &login("nathan")).unwrap();
        assert_eq!(fs::read(&outside).unwrap(), b"keep\n");
        assert_eq!(load(&state).login, Some(login("nathan")));
    }
}
