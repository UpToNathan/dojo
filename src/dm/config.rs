//! Where the display manager's files are and what it talks to, decided once at start-up.
//!
//! A real run takes everything from the installed release it was started from and from
//! fixed system paths; it has no options. A build with `fake-pam-stack` also accepts the
//! private directories and fakes the integration test needs, and no other build parses them.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use super::greeter::{Greeters, PamGreeters};
use super::host::{Fingerprints, Fprintd, Logind, Power, Seat};
use super::process::HelperSpec;
use super::vt::{Console, StdinConsole};

pub const RELEASES: &str = "/usr/local/lib/dojo-dm";
pub const SOCKET: &str = "/run/dojo-dm/greeter.sock";
pub const STATE: &str = "/var/lib/dojo-dm";

/// A root-owned release directory: `<RELEASES>/<revision>`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Release(PathBuf);

impl Release {
    /// The directory of a binary run from an installed release. `None` for a binary run
    /// from anywhere else, a checkout or a home directory above all.
    pub fn of_executable(executable: &Path) -> Option<Release> {
        let directory = executable.parent()?;
        let revision = directory.strip_prefix(RELEASES).ok()?;
        let mut parts = revision.components();
        let one_name = matches!(parts.next(), Some(Component::Normal(_))) && parts.next().is_none();
        one_name.then(|| Release(directory.to_owned()))
    }

    #[cfg(feature = "fake-pam-stack")]
    pub fn at(directory: PathBuf) -> Release {
        Release(directory)
    }

    pub fn directory(&self) -> &Path {
        &self.0
    }

    pub fn helper(&self) -> PathBuf {
        self.0.join("dojo-dm-pam")
    }

    pub fn niri(&self) -> PathBuf {
        self.0.join("niri")
    }

    pub fn greeter_config(&self) -> PathBuf {
        self.0.join("greeter.kdl")
    }

    pub fn environment(&self) -> PathBuf {
        self.0.join("environment.sh")
    }

    /// What the greeter's session runs through `/bin/sh -c`: the release's environment,
    /// then the release's Niri with the greeter's configuration.
    pub fn greeter_command(&self) -> String {
        format!(
            ". {} && exec {} --config {}",
            quote(&self.environment()),
            quote(&self.niri()),
            quote(&self.greeter_config())
        )
    }
}

fn quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

/// Everything the service reads or writes, by path.
#[derive(Clone, Debug)]
pub struct Settings {
    pub socket: PathBuf,
    pub state: PathBuf,
    pub sessions: PathBuf,
    /// The uid that must own the session directory and its entries.
    pub sessions_owner: u32,
    pub helper: HelperSpec,
}

/// The things the service does to the machine, each behind a trait.
pub struct Parts {
    pub console: Box<dyn Console>,
    pub greeters: Box<dyn Greeters>,
    pub power: Box<dyn Power>,
    pub seat: Box<dyn Seat>,
    pub fingerprints: Box<dyn Fingerprints>,
    /// The group that may read snapshots and connect to the socket.
    pub greeter_group: Option<u32>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Unusable(pub String);

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// An installed run: nothing is read from the command line or the environment.
pub fn installed(arguments: Vec<OsString>) -> Result<(Settings, Parts), Unusable> {
    #[cfg(feature = "fake-pam-stack")]
    if !arguments.is_empty() {
        return test::build(arguments);
    }
    if !arguments.is_empty() {
        return Err(Unusable("dojo-dm takes no arguments".into()));
    }
    let executable = std::env::current_exe().map_err(|e| Unusable(e.to_string()))?;
    let release = Release::of_executable(&executable).ok_or_else(|| {
        Unusable(format!(
            "{} is not inside {RELEASES}/<release>",
            executable.display()
        ))
    })?;
    let console = StdinConsole::open().map_err(|e| Unusable(e.to_string()))?;
    let greeter = super::greeter::greeter_account()
        .ok_or_else(|| Unusable("the dojo-greeter user does not exist".into()))?;
    let settings = Settings {
        socket: SOCKET.into(),
        state: STATE.into(),
        sessions: super::sessions::DIRECTORY.into(),
        sessions_owner: 0,
        helper: HelperSpec::new(release.helper()),
    };
    let parts = Parts {
        console: Box::new(console),
        greeters: Box::new(PamGreeters::new(&settings.helper, release, greeter.uid)),
        power: Box::new(Logind),
        seat: Box::new(Logind),
        fingerprints: Box::new(Fprintd),
        greeter_group: Some(greeter.gid),
    };
    Ok((settings, parts))
}

#[cfg(feature = "fake-pam-stack")]
pub mod test;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_binary_in_an_installed_release_is_at_home() {
        let release = Release::of_executable(Path::new("/usr/local/lib/dojo-dm/0123abcd/dojo-dm"));
        let release = release.unwrap();
        assert_eq!(release.helper(), Path::new("/usr/local/lib/dojo-dm/0123abcd/dojo-dm-pam"));
        for elsewhere in [
            "/home/nathan/Dojo/target/release/dojo-dm",
            "/usr/local/lib/dojo-dm/dojo-dm",
            "/usr/local/lib/dojo-dm/a/b/dojo-dm",
            "/usr/local/lib/dojo-dm/../x/dojo-dm",
            "/usr/local/lib/dojo-dm/./dojo-dm",
            "/usr/local/lib/dojo-dm-evil/a/dojo-dm",
            "/tmp/usr/local/lib/dojo-dm/a/dojo-dm",
            "dojo-dm",
            "/",
        ] {
            assert_eq!(Release::of_executable(Path::new(elsewhere)), None, "{elsewhere}");
        }
    }

    #[test]
    fn the_greeter_runs_the_releases_niri_with_the_releases_config_after_its_environment() {
        let release = Release::of_executable(Path::new("/usr/local/lib/dojo-dm/r1/dojo-dm")).unwrap();
        assert_eq!(
            release.greeter_command(),
            ". '/usr/local/lib/dojo-dm/r1/environment.sh' && exec '/usr/local/lib/dojo-dm/r1/niri' \
             --config '/usr/local/lib/dojo-dm/r1/greeter.kdl'"
        );
        assert_eq!(quote(Path::new("/a'b")), "'/a'\\''b'");
    }

    #[test]
    fn an_installed_run_takes_no_arguments() {
        #[cfg(not(feature = "fake-pam-stack"))]
        assert!(installed(vec!["--socket".into()]).is_err());
    }
}
