//! How a session is started, as data: the PAM variables that must exist before
//! `pam_open_session`, and the process the helper runs once it returns. Pure.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

use super::account::{Account, GREETER_LOGIN};
use super::launch::Start;
use crate::lock::pam::Method;

pub const SEAT: &str = "seat0";
pub const SESSION_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/bin";

/// The PAM service a `dojo-dm-pam` process runs: one of the two login methods, or the
/// greeter's own session, which no user ever authenticates against.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Service {
    Login(Method),
    Greeter,
}

impl Service {
    pub const GREETER: &'static str = GREETER_LOGIN;

    pub fn from_name(name: &str) -> Option<Service> {
        if name == Self::GREETER {
            return Some(Service::Greeter);
        }
        Method::from_login_service(name).map(Service::Login)
    }

    pub const fn name(self) -> &'static str {
        match self {
            Service::Login(method) => method.login_service(),
            Service::Greeter => Self::GREETER,
        }
    }
}

/// `pam_systemd` reads the session's type, class, desktop, seat and VT from the PAM
/// environment, and the environment wins over the module's `class=` argument, so these
/// go in before `pam_open_session`.
pub fn pam_variables(account: &Account, start: &Start) -> Vec<(&'static str, String)> {
    let class = if account.is_greeter() { "greeter" } else { "user" };
    vec![
        ("XDG_SESSION_TYPE", "wayland".into()),
        ("XDG_SESSION_CLASS", class.into()),
        ("XDG_SESSION_DESKTOP", start.desktop.as_str().into()),
        ("XDG_SEAT", SEAT.into()),
        ("XDG_VTNR", start.vtnr.to_string()),
    ]
}

/// `NAME=value` lines from `pam_getenvlist`. A line without `=` or with an empty name is dropped.
pub fn parse_pam_environment(lines: Vec<Vec<u8>>) -> Vec<(OsString, OsString)> {
    lines
        .into_iter()
        .filter_map(|line| {
            let split = line.iter().position(|&byte| byte == b'=')?;
            (split > 0).then(|| {
                (
                    OsString::from_vec(line[..split].to_vec()),
                    OsString::from_vec(line[split + 1..].to_vec()),
                )
            })
        })
        .collect()
}

/// Everything needed to start the session process, with the privilege drop left to the caller.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SessionPlan {
    pub shell: PathBuf,
    pub arguments: Vec<OsString>,
    pub environment: Vec<(OsString, OsString)>,
    pub home: PathBuf,
    pub uid: u32,
    pub gid: u32,
    pub account: std::ffi::CString,
}

/// The login shell runs `exec <Exec>`: the shell's profile sets up the user's environment,
/// then the shell is replaced by the session. The greeter has no profile: `/bin/sh` runs the
/// display manager's own command line as given. The environment is built from nothing:
/// the account, the session, then whatever PAM added, which wins.
pub fn plan(
    account: &Account,
    start: &Start,
    pam_environment: Vec<(OsString, OsString)>,
    shell: Option<PathBuf>,
) -> SessionPlan {
    let mut environment: BTreeMap<OsString, OsString> = BTreeMap::new();
    let mut set = |name: &str, value: &OsStr| {
        environment.insert(name.into(), value.to_owned());
    };
    set("HOME", account.home.as_os_str());
    set("USER", account.login.as_str().as_ref());
    set("LOGNAME", account.login.as_str().as_ref());
    set("SHELL", account.shell.as_os_str());
    set("PATH", SESSION_PATH.as_ref());
    set(
        "XDG_CURRENT_DESKTOP",
        start.desktop_names.join(":").as_ref(),
    );
    for (name, value) in pam_variables(account, start) {
        set(name, value.as_ref());
    }
    environment.extend(pam_environment);
    // The kernel and PAM may not set the empty name; nothing may set a name that is not a name.
    environment.retain(|name, _| !name.is_empty() && !name.as_bytes().contains(&b'='));
    SessionPlan {
        shell: shell.unwrap_or_else(|| account.shell.clone()),
        arguments: if account.is_greeter() {
            vec!["-c".into(), start.exec.clone().into()]
        } else {
            vec![
                "-l".into(),
                "-c".into(),
                format!("exec {}", start.exec).into(),
            ]
        },
        environment: environment.into_iter().collect(),
        home: account.home.clone(),
        uid: account.uid,
        gid: account.gid,
        account: account.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dm::protocol::{Login, SessionId};

    fn account() -> Account {
        Account {
            login: Login::parse("nathan").unwrap(),
            name: c"nathan".into(),
            uid: 1000,
            gid: 1000,
            home: "/home/nathan".into(),
            shell: "/usr/bin/fish".into(),
        }
    }

    fn start() -> Start {
        Start {
            exec: "uwsm start -F -- niri".into(),
            desktop: SessionId::parse("niri-uwsm").unwrap(),
            desktop_names: vec!["niri".into(), "GNOME".into()],
            vtnr: 1,
            tty: "tty1".into(),
        }
    }

    fn env(plan: &SessionPlan, name: &str) -> Option<String> {
        plan.environment
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    }

    #[test]
    fn pam_sees_the_session_identity_before_it_opens() {
        assert_eq!(
            pam_variables(&account(), &start()),
            [
                ("XDG_SESSION_TYPE", "wayland".to_owned()),
                ("XDG_SESSION_CLASS", "user".to_owned()),
                ("XDG_SESSION_DESKTOP", "niri-uwsm".to_owned()),
                ("XDG_SEAT", "seat0".to_owned()),
                ("XDG_VTNR", "1".to_owned()),
            ]
        );
    }

    fn greeter() -> Account {
        Account {
            login: Login::parse("dojo-greeter").unwrap(),
            name: c"dojo-greeter".into(),
            uid: 975,
            gid: 975,
            home: "/".into(),
            shell: "/bin/sh".into(),
        }
    }

    #[test]
    fn services_name_the_pam_stacks_and_nothing_else() {
        for (name, service) in [
            ("dojo-login", Service::Login(Method::Password)),
            ("dojo-login-fingerprint", Service::Login(Method::Fingerprint)),
            ("dojo-greeter", Service::Greeter),
        ] {
            assert_eq!(Service::from_name(name), Some(service));
            assert_eq!(service.name(), name);
        }
        for stranger in ["dojo-lock-password", "system-auth", "", "dojo-greeter ", "sudo"] {
            assert_eq!(Service::from_name(stranger), None, "{stranger:?}");
        }
    }

    #[test]
    fn the_greeter_opens_a_greeter_class_session_and_runs_its_command_line_directly() {
        let mut start = start();
        start.exec = ". /usr/local/lib/dojo-dm/r/environment.sh && exec /usr/local/lib/dojo-dm/r/niri".into();
        start.desktop = SessionId::parse("dojo-greeter").unwrap();
        start.desktop_names.clear();
        let class = pam_variables(&greeter(), &start)
            .into_iter()
            .find(|(name, _)| *name == "XDG_SESSION_CLASS")
            .map(|(_, value)| value);
        assert_eq!(class.as_deref(), Some("greeter"));
        let plan = plan(&greeter(), &start, vec![], None);
        assert_eq!(plan.shell, PathBuf::from("/bin/sh"));
        assert_eq!(plan.arguments, [OsString::from("-c"), start.exec.clone().into()]);
        assert_eq!(env(&plan, "XDG_SESSION_CLASS").as_deref(), Some("greeter"));
        assert_eq!(env(&plan, "XDG_SESSION_DESKTOP").as_deref(), Some("dojo-greeter"));
        assert_eq!(env(&plan, "XDG_VTNR").as_deref(), Some("1"));
        assert_eq!(env(&plan, "SHELL").as_deref(), Some("/bin/sh"));
    }

    #[test]
    fn the_shell_execs_the_session_as_a_login_shell() {
        let plan = plan(&account(), &start(), vec![], None);
        assert_eq!(plan.shell, PathBuf::from("/usr/bin/fish"));
        assert_eq!(
            plan.arguments,
            ["-l", "-c", "exec uwsm start -F -- niri"].map(OsString::from)
        );
        assert_eq!((plan.uid, plan.gid), (1000, 1000));
        assert_eq!(plan.home, PathBuf::from("/home/nathan"));
    }

    #[test]
    fn the_environment_is_built_from_nothing() {
        let plan = plan(&account(), &start(), vec![], None);
        let expected = [
            ("HOME", "/home/nathan"),
            ("LOGNAME", "nathan"),
            ("PATH", SESSION_PATH),
            ("SHELL", "/usr/bin/fish"),
            ("USER", "nathan"),
            ("XDG_CURRENT_DESKTOP", "niri:GNOME"),
            ("XDG_SEAT", "seat0"),
            ("XDG_SESSION_CLASS", "user"),
            ("XDG_SESSION_DESKTOP", "niri-uwsm"),
            ("XDG_SESSION_TYPE", "wayland"),
            ("XDG_VTNR", "1"),
        ];
        let actual: Vec<(String, String)> = plan
            .environment
            .iter()
            .map(|(k, v)| (k.to_string_lossy().into(), v.to_string_lossy().into()))
            .collect();
        assert_eq!(
            actual,
            expected
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn pam_additions_win_and_malformed_ones_are_dropped() {
        let pam = parse_pam_environment(vec![
            b"XDG_RUNTIME_DIR=/run/user/1000".to_vec(),
            b"PATH=/pam/bin".to_vec(),
            b"EMPTY=".to_vec(),
            b"NOEQUALS".to_vec(),
            b"=novalue".to_vec(),
            b"A=b=c".to_vec(),
        ]);
        assert_eq!(pam.len(), 4);
        let plan = plan(&account(), &start(), pam, None);
        assert_eq!(
            env(&plan, "XDG_RUNTIME_DIR").as_deref(),
            Some("/run/user/1000")
        );
        assert_eq!(env(&plan, "PATH").as_deref(), Some("/pam/bin"));
        assert_eq!(env(&plan, "EMPTY").as_deref(), Some(""));
        assert_eq!(env(&plan, "A").as_deref(), Some("b=c"));
        assert_eq!(env(&plan, "NOEQUALS"), None);
        assert_eq!(env(&plan, "HOME").as_deref(), Some("/home/nathan"));
    }

    #[test]
    fn an_empty_desktop_names_list_still_sets_the_variable() {
        let mut bare = start();
        bare.desktop_names.clear();
        let plan = plan(&account(), &bare, vec![], None);
        assert_eq!(env(&plan, "XDG_CURRENT_DESKTOP").as_deref(), Some(""));
    }
}
