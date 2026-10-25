//! Reaching a peer: who may be reached, what is run there and how a refusal is told apart from a
//! failure. Pure; the process that carries it out is in `ssh`.

use crate::roster::{Member, Roster, Source};
use serde::{Deserialize, Serialize};

/// Who started a remote call. The agent's calls are marked so a peer and the audit log can tell
/// them from the operator's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    Operator,
    Agent,
}

impl Origin {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "operator" => Some(Self::Operator),
            "agent" => Some(Self::Agent),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Agent => "agent",
        }
    }
}

/// Who is calling, handed to the peer as environment variables of the remote command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Provenance {
    pub from: String,
    pub session: String,
    pub origin: Origin,
}

/// Single-quotes `arg` for a POSIX shell.
pub fn quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "'\\''"))
}

/// The one string sent to the peer's shell: `env` with the provenance, then the quoted argv. The
/// arguments are never interpreted by that shell, only passed on.
pub fn remote_command(argv: &[String], provenance: &Provenance) -> Result<String, String> {
    let Some(first) = argv.first() else { return Err("there is no command to run".into()) };
    if first.is_empty() {
        return Err("the command name is empty".into());
    }
    // `env` reads a leading NAME=value as an assignment and a leading dash as its own option, and
    // has no way to end either, so such a command name cannot be passed faithfully.
    if first.contains('=') || first.starts_with('-') {
        return Err(format!("the command name {first:?} cannot start with a dash or contain '='; run it through sh -c"));
    }
    if argv.iter().any(|arg| arg.contains('\0')) {
        return Err("a command argument contains a NUL byte".into());
    }
    let mut parts = vec![
        "env".to_string(),
        format!("DOJO_FROM={}", quote(&provenance.from)),
        format!("DOJO_SESSION={}", quote(&provenance.session)),
        format!("DOJO_ORIGIN={}", provenance.origin.as_str()),
    ];
    parts.extend(argv.iter().map(|arg| quote(arg)));
    Ok(parts.join(" "))
}

/// Why a call was refused before anything was sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The network cannot be read at all.
    NoNetwork(String),
    /// Not a member of this node's mesh (unknown, not admitted or denied).
    NotMember(String),
    /// Names this node itself.
    ThisNode,
    Offline(String),
    /// The agent's peer access is switched off.
    Paused,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoNetwork(reason) => write!(f, "the network cannot be read ({reason})"),
            Self::NotMember(name) => write!(f, "{name} is not a member of this mesh (see `dojo-mesh status --all`)"),
            Self::ThisNode => f.write_str("that is this node; run the command directly"),
            Self::Offline(name) => write!(f, "{name} is offline"),
            Self::Paused => f.write_str("peer access by the agent is paused (`dojo-mesh resume` to allow it)"),
        }
    }
}

/// The member a name refers to, if it can be reached now. A peer that is offline fails here, at
/// once, rather than after a connection timeout.
pub fn resolve<'a>(roster: &'a Roster, name: &str) -> Result<&'a Member, Refusal> {
    if let Source::Unavailable(reason) = &roster.source {
        return Err(Refusal::NoNetwork(reason.clone()));
    }
    let key = crate::config::name_key(name);
    if roster.this().answers_to(&key) {
        return Err(Refusal::ThisNode);
    }
    let member = roster.peers().find(|member| member.device.answers_to(&key)).ok_or_else(|| Refusal::NotMember(name.to_string()))?;
    if !member.device.online {
        return Err(Refusal::Offline(member.device.name.clone()));
    }
    Ok(member)
}

/// How to reach a member: its full network name when there is one.
pub fn route(member: &Member) -> crate::method::Route {
    crate::method::Route::Host { name: host(member), port: None }
}

/// The name to connect to: the full network name when there is one.
pub fn host(member: &Member) -> String {
    member.device.dns_name.as_deref().map(|dns| dns.trim_end_matches('.').to_string()).unwrap_or_else(|| member.device.name.clone())
}

/// The sign-in page Tailscale SSH prints when a policy asks a human to re-authenticate, found in a
/// line of the client's error output.
pub fn check_url(line: &str) -> Option<String> {
    let start = line.find("https://login.tailscale.com/a/")?;
    let url: String = line[start..].chars().take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'').collect();
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MeshConfig;
    use crate::device::device;
    use crate::roster::admit;

    fn provenance() -> Provenance {
        Provenance { from: "alpha".into(), session: "s1".into(), origin: Origin::Agent }
    }

    fn roster() -> Roster {
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"").unwrap();
        admit(
            device("alpha", &["tag:mesh"], true),
            vec![device("beta", &["tag:mesh"], true), device("gone", &["tag:mesh"], false), device("stranger", &[], true)],
            &config,
        )
    }

    #[test]
    fn quoting_keeps_every_argument_literal() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("$(rm -rf ~); `x` \"y\" \n z"), "'$(rm -rf ~); `x` \"y\" \n z'");
    }

    #[test]
    fn the_remote_command_carries_provenance_and_quoted_arguments() {
        let command = remote_command(&["echo".into(), "a b".into(), "'; id".into()], &provenance()).unwrap();
        assert_eq!(command, "env DOJO_FROM='alpha' DOJO_SESSION='s1' DOJO_ORIGIN=agent 'echo' 'a b' ''\\''; id'");
    }

    #[test]
    fn rejects_commands_that_cannot_be_sent_faithfully() {
        assert!(remote_command(&[], &provenance()).is_err());
        assert!(remote_command(&["".into()], &provenance()).is_err());
        assert!(remote_command(&["-S".into()], &provenance()).is_err());
        assert!(remote_command(&["A=1".into(), "x".into()], &provenance()).is_err());
        assert!(remote_command(&["a".into(), "b\0c".into()], &provenance()).is_err());
    }

    #[test]
    fn resolves_members_by_short_or_full_name() {
        let roster = roster();
        assert_eq!(resolve(&roster, "BETA").unwrap().device.name, "beta");
        assert_eq!(resolve(&roster, "beta.example.ts.net.").unwrap().device.name, "beta");
        assert_eq!(host(resolve(&roster, "beta").unwrap()), "beta.example.ts.net");
    }

    #[test]
    fn refuses_before_connecting() {
        let roster = roster();
        assert_eq!(resolve(&roster, "gone").unwrap_err(), Refusal::Offline("gone".into()));
        assert_eq!(resolve(&roster, "stranger").unwrap_err(), Refusal::NotMember("stranger".into()));
        assert_eq!(resolve(&roster, "nobody").unwrap_err(), Refusal::NotMember("nobody".into()));
        assert_eq!(resolve(&roster, "alpha").unwrap_err(), Refusal::ThisNode);
        let alone = Roster::alone(device("alpha", &[], true), &MeshConfig::default(), "no tailscale".into());
        assert!(matches!(resolve(&alone, "beta"), Err(Refusal::NoNetwork(_))));
    }

    #[test]
    fn finds_the_check_page_in_the_clients_message() {
        assert_eq!(
            check_url("To authenticate, visit: https://login.tailscale.com/a/abc123def and try again"),
            Some("https://login.tailscale.com/a/abc123def".into())
        );
        assert_eq!(check_url("ssh: connect to host failed"), None);
    }
}
