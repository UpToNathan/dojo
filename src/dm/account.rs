//! Who may log in: a name from the greeter checked against the passwd database.
//! The policy is pure; `lookup` and `read_shells` are the only host access.

use std::ffi::{CStr, CString};
use std::path::{Path, PathBuf};

use super::protocol::Login;

/// Human accounts. System users and the nobody range never get a login screen session.
pub const MIN_UID: u32 = 1000;
pub const MAX_UID: u32 = 60000;

/// One passwd entry as the database returned it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Entry {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    pub shell: PathBuf,
}

/// An account the display manager has agreed to start a session for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Account {
    pub login: Login,
    pub name: CString,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
    pub shell: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    Unknown,
    /// Outside `MIN_UID..MAX_UID`, so root, system users and nobody.
    NotHuman,
    BadShell,
    BadHome,
}

impl Account {
    /// `shells` is /etc/shells; `is_executable` answers for a shell it does not list.
    pub fn validate(
        login: &Login,
        entry: Option<Entry>,
        shells: &[PathBuf],
        is_executable: impl Fn(&Path) -> bool,
    ) -> Result<Account, Refusal> {
        let entry = entry
            .filter(|entry| entry.name == login.as_str())
            .ok_or(Refusal::Unknown)?;
        if !(MIN_UID..MAX_UID).contains(&entry.uid) {
            return Err(Refusal::NotHuman);
        }
        if !shell_acceptable(&entry.shell, shells, is_executable) {
            return Err(Refusal::BadShell);
        }
        if !entry.home.is_absolute() {
            return Err(Refusal::BadHome);
        }
        Ok(Account {
            name: CString::new(entry.name).map_err(|_| Refusal::Unknown)?,
            login: login.clone(),
            uid: entry.uid,
            gid: entry.gid,
            home: entry.home,
            shell: entry.shell,
        })
    }
}

/// An absolute path that is not a refusal shell, and is either listed in /etc/shells
/// or an executable file. `nologin` and `false` are refused even when listed.
fn shell_acceptable(
    shell: &Path,
    listed: &[PathBuf],
    is_executable: impl Fn(&Path) -> bool,
) -> bool {
    let refusal = matches!(
        shell.file_name().and_then(|name| name.to_str()),
        None | Some("nologin" | "false")
    );
    shell.is_absolute() && !refusal && (listed.iter().any(|l| l == shell) || is_executable(shell))
}

/// Entries of /etc/shells: absolute paths, comments and blanks skipped.
pub fn parse_shells(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| line.starts_with('/'))
        .map(PathBuf::from)
        .collect()
}

pub fn read_shells() -> Vec<PathBuf> {
    std::fs::read_to_string("/etc/shells")
        .map(|text| parse_shells(&text))
        .unwrap_or_default()
}

pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// `getpwnam_r`. `None` for an unknown user or a database error.
pub fn lookup(login: &Login) -> Option<Entry> {
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(login.as_str()).ok()?;
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut found = std::ptr::null_mut();
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    loop {
        let rc = unsafe {
            libc::getpwnam_r(
                name.as_ptr(),
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        };
        match rc {
            0 => break,
            libc::ERANGE if buffer.len() < 1 << 20 => buffer.resize(buffer.len() * 2, 0),
            _ => return None,
        }
    }
    if found.is_null() || entry.pw_name.is_null() || entry.pw_dir.is_null() {
        return None;
    }
    let field = |pointer: *const libc::c_char| {
        (!pointer.is_null()).then(|| unsafe { CStr::from_ptr(pointer) }.to_bytes())
    };
    Some(Entry {
        name: String::from_utf8(field(entry.pw_name)?.to_vec()).ok()?,
        uid: entry.pw_uid,
        gid: entry.pw_gid,
        home: PathBuf::from(std::ffi::OsStr::from_bytes(field(entry.pw_dir)?)),
        shell: PathBuf::from(std::ffi::OsStr::from_bytes(
            field(entry.pw_shell).unwrap_or(b""),
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn login(text: &str) -> Login {
        Login::parse(text).unwrap()
    }

    fn entry(name: &str, uid: u32, shell: &str) -> Entry {
        Entry {
            name: name.into(),
            uid,
            gid: 1000,
            home: format!("/home/{name}").into(),
            shell: shell.into(),
        }
    }

    fn shells() -> Vec<PathBuf> {
        parse_shells("# comment\n/bin/bash\n\n  /usr/bin/fish  \n/usr/bin/nologin\nnot-absolute\n")
    }

    fn check(entry: Option<Entry>) -> Result<Account, Refusal> {
        let name = entry
            .as_ref()
            .map_or("nathan".to_owned(), |e| e.name.clone());
        Account::validate(&login(&name), entry, &shells(), |path| {
            path == Path::new("/nix/profile/bin/zsh")
        })
    }

    #[test]
    fn a_human_with_a_listed_shell_is_accepted() {
        let account = check(Some(entry("nathan", 1000, "/usr/bin/fish"))).unwrap();
        assert_eq!(account.uid, 1000);
        assert_eq!(account.name.to_str(), Ok("nathan"));
        assert_eq!(account.home, Path::new("/home/nathan"));
        assert_eq!(account.shell, Path::new("/usr/bin/fish"));
    }

    #[test]
    fn the_uid_range_is_half_open() {
        for (uid, ok) in [
            (0, false),
            (999, false),
            (1000, true),
            (59999, true),
            (60000, false),
            (65534, false),
        ] {
            let result = check(Some(entry("nathan", uid, "/bin/bash")));
            assert_eq!(result.is_ok(), ok, "{uid}");
            if !ok {
                assert_eq!(result, Err(Refusal::NotHuman));
            }
        }
    }

    #[test]
    fn shells_are_listed_or_executable_and_never_nologin() {
        let shell = |path: &str| check(Some(entry("nathan", 1000, path))).map(|a| a.shell);
        assert!(shell("/bin/bash").is_ok());
        assert!(
            shell("/nix/profile/bin/zsh").is_ok(),
            "executable but unlisted"
        );
        for bad in [
            "",
            "bash",
            "/usr/bin/nologin",
            "/sbin/nologin",
            "/bin/false",
            "/opt/unlisted/bin/sh",
            "/",
        ] {
            assert_eq!(shell(bad), Err(Refusal::BadShell), "{bad:?}");
        }
    }

    #[test]
    fn unknown_mismatched_and_homeless_accounts_are_refused() {
        assert_eq!(check(None), Err(Refusal::Unknown));
        assert_eq!(
            Account::validate(
                &login("alice"),
                Some(entry("bob", 1000, "/bin/bash")),
                &shells(),
                |_| false
            ),
            Err(Refusal::Unknown)
        );
        let mut homeless = entry("nathan", 1000, "/bin/bash");
        homeless.home = "relative".into();
        assert_eq!(check(Some(homeless)), Err(Refusal::BadHome));
    }

    #[test]
    fn etc_shells_keeps_only_absolute_paths() {
        assert_eq!(
            shells(),
            [
                PathBuf::from("/bin/bash"),
                PathBuf::from("/usr/bin/fish"),
                PathBuf::from("/usr/bin/nologin")
            ]
        );
    }

    #[test]
    fn the_passwd_database_answers_for_the_current_user_and_not_for_nobody() {
        let own = unsafe { libc::getuid() };
        let entry = {
            let passwd = unsafe { libc::getpwuid(own) };
            assert!(!passwd.is_null());
            unsafe { CStr::from_ptr((*passwd).pw_name) }
                .to_str()
                .unwrap()
                .to_owned()
        };
        if let Some(login) = Login::parse(&entry) {
            assert_eq!(lookup(&login).map(|found| found.uid), Some(own));
        }
        assert_eq!(lookup(&login("no-such-user-dojo")), None);
    }
}
