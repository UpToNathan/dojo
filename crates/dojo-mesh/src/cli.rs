//! The `dojo-mesh` command line. Everything it needs from the machine comes in through [`Env`], so
//! the commands read the same wherever they run.
//!
//! Exit codes: 0 done; 1 the command ran and failed (the peer's own status is in the message);
//! 2 usage or configuration; 3 a human has to act (Tailscale wants a sign-in, or an operator-only
//! step has no operator); 4 the call was refused or could not reach the peer.

use crate::audit::{self, Entry};
use crate::capability::{self, Answer, Description};
use crate::config::{MeshConfig, default_path};
use crate::doctor::{Severity, diagnose};
use crate::roster::Roster;
use crate::ssh::{self, Call, Done, Ended, Input, Output};
use crate::transport::{self, Origin, Provenance, Refusal};
use crate::{guard, report, tailscale};
use chrono::Utc;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

pub const FAILED: u8 = 1;
pub const USAGE_ERROR: u8 = 2;
pub const NEEDS_HUMAN: u8 = 3;
pub const UNAVAILABLE: u8 = 4;

const USAGE: &str = "\
usage: dojo-mesh [--config PATH] <command>
  status [--all] [--json]          this node and the peers it admits
  doctor [--json]                  what is wrong or doubtful
  describe                         what this node offers, as JSON
  find CAPABILITY [--json | --tsv] [--peers-only]
                                   which nodes offer a capability (--tsv: node, host, key=value... per line)
  exec [--timeout SECS] PEER -- COMMAND [ARG...]
                                   run a command on a peer
  cp SRC DST                       copy one file; exactly one side is PEER:PATH
  task PEER TEXT...                give a peer's agent a task
  pause | resume                   switch the agent's peer access off or on
  audit [--last N] [--json]        what this node has sent to peers
  enroll [--tag TAG] [--expires SECS]
                                   mint a one-use key for a new node (operator only)
  policy show|validate FILE|apply FILE
                                   read, check or replace the network policy (apply is operator only)
";

pub struct Failure {
    pub code: u8,
    pub message: String,
}

pub fn fail(code: u8, message: impl Into<String>) -> Failure {
    Failure { code, message: message.into() }
}

/// What the process learns from the machine it runs on.
pub struct Env {
    pub config_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub home: Option<PathBuf>,
    pub hostname: String,
    pub origin: Origin,
    pub session: String,
}

impl Env {
    pub fn from_process(config_override: Option<PathBuf>) -> Result<Self, Failure> {
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let home = var("HOME");
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname").map(|name| name.trim().to_string()).unwrap_or_default();
        let origin = match std::env::var("DOJO_MESH_ORIGIN") {
            Err(_) => Origin::Operator,
            Ok(text) => Origin::parse(&text).ok_or_else(|| fail(USAGE_ERROR, format!("DOJO_MESH_ORIGIN must be operator or agent, not {text:?}")))?,
        };
        let session = std::env::var("DOJO_MESH_SESSION").ok().filter(|id| !id.is_empty()).unwrap_or_else(random_id);
        Ok(Self {
            config_path: config_override.or_else(|| default_path(var("XDG_CONFIG_HOME"), home.clone())),
            state_dir: guard::state_dir(var("XDG_STATE_HOME"), home.clone()),
            home,
            hostname: if hostname.is_empty() { "this-node".into() } else { hostname },
            origin,
            session,
        })
    }
}

fn random_id() -> String {
    let mut bytes = [0u8; 6];
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut file, &mut bytes);
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn main(mut args: Vec<String>) -> ExitCode {
    let result = (|| {
        let mut config_override = None;
        if args.first().map(String::as_str) == Some("--config") {
            if args.len() < 2 {
                return Err(fail(USAGE_ERROR, format!("--config needs a path\n{USAGE}")));
            }
            config_override = Some(PathBuf::from(args[1].clone()));
            args.drain(..2);
        }
        let env = Env::from_process(config_override)?;
        run(&env, &args)
    })();
    match result {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            eprintln!("dojo-mesh: {}", failure.message);
            ExitCode::from(failure.code)
        }
    }
}

/// Flags that take no value, separated from the positional words.
struct Words {
    flags: Vec<String>,
    positional: Vec<String>,
}

fn split(args: &[String], switches: &[&str], valued: &[&str]) -> Result<(Words, Vec<(String, String)>), Failure> {
    let mut words = Words { flags: Vec::new(), positional: Vec::new() };
    let mut values = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if switches.contains(&arg.as_str()) {
            words.flags.push(arg.clone());
        } else if valued.contains(&arg.as_str()) {
            let value = iter.next().ok_or_else(|| fail(USAGE_ERROR, format!("{arg} needs a value")))?;
            values.push((arg.clone(), value.clone()));
        } else if arg.starts_with("--") {
            return Err(fail(USAGE_ERROR, format!("unknown option {arg}\n{USAGE}")));
        } else {
            words.positional.push(arg.clone());
        }
    }
    Ok((words, values))
}

fn has(words: &Words, flag: &str) -> bool {
    words.flags.iter().any(|candidate| candidate == flag)
}

fn load_config(env: &Env) -> Result<MeshConfig, Failure> {
    match &env.config_path {
        Some(path) => MeshConfig::load(path).map_err(|error| fail(USAGE_ERROR, error.to_string())),
        None => Ok(MeshConfig::default()),
    }
}

fn load_roster(env: &Env, config: &MeshConfig) -> Roster {
    tailscale::roster(tailscale::read_status(), config, &env.hostname)
}

fn json<T: serde::Serialize>(value: &T) -> Result<String, Failure> {
    serde_json::to_string_pretty(value).map_err(|error| fail(FAILED, error.to_string()))
}

pub fn run(env: &Env, args: &[String]) -> Result<u8, Failure> {
    let Some(command) = args.first() else { return Err(fail(USAGE_ERROR, USAGE.trim_end())) };
    let rest = &args[1..];
    match command.as_str() {
        "status" => {
            let (words, _) = split(rest, &["--all", "--json"], &[])?;
            let config = load_config(env)?;
            let roster = load_roster(env, &config);
            if has(&words, "--json") {
                println!("{}", json(&roster)?);
            } else {
                print!("{}", report::status(&roster, has(&words, "--all")));
                if env.state_dir.as_deref().is_some_and(guard::agent_paused) {
                    println!("  agent peer access: paused");
                }
            }
            Ok(0)
        }
        "doctor" => {
            let (words, _) = split(rest, &["--json"], &[])?;
            let config = load_config(env)?;
            let roster = load_roster(env, &config);
            let findings = diagnose(&roster, &config, Utc::now());
            if has(&words, "--json") {
                println!("{}", json(&findings)?);
            } else {
                print!("{}", report::doctor(&findings));
            }
            Ok(if findings.iter().any(|finding| finding.severity == Severity::Problem) { 1 } else { 0 })
        }
        "describe" => {
            split(rest, &[], &[])?;
            let config = load_config(env)?;
            println!("{}", json(&describe_here(env, &config, &env.hostname))?);
            Ok(0)
        }
        "find" => find(env, rest),
        "exec" => exec(env, rest),
        "cp" => copy(env, rest),
        "task" => task(env, rest),
        "pause" | "resume" => {
            split(rest, &[], &[])?;
            let dir = env.state_dir.as_deref().ok_or_else(|| fail(FAILED, "there is no state directory (HOME is not set)"))?;
            guard::set_agent_paused(dir, command == "pause").map_err(|error| fail(FAILED, error.to_string()))?;
            println!("{}", if command == "pause" { "The agent's peer access is paused." } else { "The agent's peer access is on." });
            Ok(0)
        }
        "audit" => audit_command(env, rest),
        "enroll" | "policy" => crate::control_cli::run(env, command, rest),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(0)
        }
        other => Err(fail(USAGE_ERROR, format!("unknown command {other}\n{USAGE}"))),
    }
}

fn describe_here(env: &Env, config: &MeshConfig, name: &str) -> Description {
    capability::describe(name, &config.capabilities, env.home.as_deref(), |path| std::fs::read_to_string(path).ok())
}

fn audit_command(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let (words, values) = split(rest, &["--json"], &["--last"])?;
    let count = match values.iter().find(|(flag, _)| flag == "--last") {
        Some((_, text)) => text.parse::<usize>().map_err(|_| fail(USAGE_ERROR, format!("--last needs a number, not {text:?}")))?,
        None => 20,
    };
    let dir = env.state_dir.as_deref().ok_or_else(|| fail(FAILED, "there is no state directory (HOME is not set)"))?;
    let entries = audit::last(dir, count).map_err(|error| fail(FAILED, error.to_string()))?;
    if has(&words, "--json") {
        println!("{}", json(&entries)?);
    } else if entries.is_empty() {
        println!("Nothing has been sent to a peer from this node.");
    } else {
        for entry in &entries {
            println!("{}", audit::render(entry));
        }
    }
    Ok(0)
}

// ---- calling peers --------------------------------------------------------------------------

fn record(env: &Env, from: &str, peer: &str, action: &str, detail: &str, result: &str) -> Result<(), Failure> {
    let Some(dir) = env.state_dir.as_deref() else {
        return match env.origin {
            Origin::Agent => Err(fail(UNAVAILABLE, "the agent's peer calls are logged, and there is nowhere to log (HOME is not set)")),
            Origin::Operator => Ok(()),
        };
    };
    let entry = Entry::new(env.origin, from, &env.session, peer, action, detail, result, Utc::now());
    match audit::append(dir, &entry) {
        Ok(()) => Ok(()),
        Err(error) if env.origin == Origin::Agent => Err(fail(UNAVAILABLE, format!("the agent's peer calls are logged, and the log cannot be written: {error}"))),
        Err(error) => {
            eprintln!("dojo-mesh: warning: could not write the audit log: {error}");
            Ok(())
        }
    }
}

fn refuse(env: &Env, from: &str, peer: &str, action: &str, detail: &str, why: &Refusal) -> Failure {
    let _ = record(env, from, peer, action, detail, &format!("refused: {why}"));
    fail(UNAVAILABLE, format!("{peer}: {why}"))
}

/// One call to one peer, from the checks to the record of how it ended.
#[allow(clippy::too_many_arguments)]
fn call_peer(
    env: &Env,
    roster: &Roster,
    peer: &str,
    action: &str,
    detail: &str,
    argv: &[String],
    input: Input,
    output: Output,
    timeout: Option<Duration>,
) -> Result<Done, Failure> {
    let from = roster.this().name.clone();
    if env.origin == Origin::Agent && env.state_dir.as_deref().is_some_and(guard::agent_paused) {
        return Err(refuse(env, &from, peer, action, detail, &Refusal::Paused));
    }
    let member = transport::resolve(roster, peer).map_err(|why| refuse(env, &from, peer, action, detail, &why))?;
    let provenance = Provenance { from: from.clone(), session: env.session.clone(), origin: env.origin };
    let remote = transport::remote_command(argv, &provenance).map_err(|message| fail(USAGE_ERROR, message))?;
    let name = member.device.name.clone();
    record(env, &from, &name, action, detail, "started")?;
    let done = match ssh::run(Call { host: transport::host(member), remote, input, output, timeout }) {
        Ok(done) => done,
        Err(message) => {
            let _ = record(env, &from, &name, action, detail, &format!("failed: {message}"));
            return Err(fail(UNAVAILABLE, format!("{name}: {message}")));
        }
    };
    let result = match &done.ended {
        Ended::Exited(code) => format!("exit {code}"),
        Ended::Killed => "killed".into(),
        Ended::NeedsHuman(url) => format!("needs-human {url}"),
        Ended::TimedOut => "timed-out".into(),
    };
    let _ = record(env, &from, &name, action, detail, &result);
    Ok(done)
}

/// Turns how a call ended into the command's exit status.
fn outcome(peer: &str, ended: &Ended) -> Result<u8, Failure> {
    match ended {
        Ended::Exited(0) => Ok(0),
        Ended::Exited(code) => Err(fail(FAILED, format!("{peer} exited with status {code}"))),
        Ended::Killed => Err(fail(FAILED, format!("the call to {peer} was ended by a signal"))),
        Ended::TimedOut => Err(fail(UNAVAILABLE, format!("the call to {peer} timed out"))),
        Ended::NeedsHuman(url) => Err(fail(NEEDS_HUMAN, format!("{peer} needs a person to approve this connection: open {url}, then run the command again"))),
    }
}

fn exec(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let Some(dashes) = rest.iter().position(|arg| arg == "--") else {
        return Err(fail(USAGE_ERROR, format!("exec needs -- before the command\n{USAGE}")));
    };
    let (words, values) = split(&rest[..dashes], &[], &["--timeout"])?;
    let [peer] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, format!("exec takes one peer name\n{USAGE}"))) };
    let argv = &rest[dashes + 1..];
    let timeout = match values.iter().find(|(flag, _)| flag == "--timeout") {
        Some((_, text)) => Some(Duration::from_secs(text.parse::<u64>().map_err(|_| fail(USAGE_ERROR, format!("--timeout needs whole seconds, not {text:?}")))?)),
        None => None,
    };
    let config = load_config(env)?;
    let roster = load_roster(env, &config);
    let done = call_peer(env, &roster, peer, "exec", &argv.join(" "), argv, Input::Inherit, Output::Inherit, timeout)?;
    outcome(peer, &done.ended)
}

fn task(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let (words, _) = split(rest, &[], &[])?;
    let [peer, text @ ..] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, format!("task takes a peer and the task text\n{USAGE}"))) };
    if text.is_empty() {
        return Err(fail(USAGE_ERROR, format!("task needs the task text\n{USAGE}")));
    }
    let config = load_config(env)?;
    let roster = load_roster(env, &config);
    let description = probe(env, &roster, peer)?;
    let command = description
        .capabilities
        .get("voice-agent")
        .and_then(|attributes| attributes.get("command"))
        .ok_or_else(|| fail(FAILED, format!("{peer} does not offer a voice-agent with a command; it declares none")))?;
    let argv = vec![command.clone(), "send".to_string(), text.join(" ")];
    let done = call_peer(env, &roster, peer, "task", &text.join(" "), &argv, Input::Null, Output::Inherit, None)?;
    outcome(peer, &done.ended)
}

// ---- describing and finding -----------------------------------------------------------------

/// Where a peer keeps `dojo-mesh`: next to the other installed helpers.
const REMOTE_DESCRIBE: &str = "exec \"${XDG_DATA_HOME:-$HOME/.local/share}/dojo/bin/dojo-mesh\" describe";

fn probe(env: &Env, roster: &Roster, peer: &str) -> Result<Description, Failure> {
    let argv = vec!["sh".to_string(), "-c".to_string(), REMOTE_DESCRIBE.to_string()];
    let done = call_peer(env, roster, peer, "describe", "", &argv, Input::Null, Output::Capture, Some(Duration::from_secs(8)))?;
    outcome(peer, &done.ended)?;
    serde_json::from_slice(&done.stdout).map_err(|error| fail(FAILED, format!("{peer} gave a description that could not be read: {error}")))
}

fn find(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let (words, _) = split(rest, &["--json", "--tsv", "--peers-only"], &[])?;
    let [wanted] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, format!("find takes one capability name\n{USAGE}"))) };
    let config = load_config(env)?;
    let roster = load_roster(env, &config);
    let mut answers = Vec::new();
    if !has(&words, "--peers-only") {
        let this = roster.this();
        answers.push(Answer {
            node: this.name.clone(),
            host: this.dns_name.clone().map(|dns| dns.trim_end_matches('.').to_string()).unwrap_or_else(|| this.name.clone()),
            this_node: true,
            description: Ok(describe_here(env, &config, &this.name)),
        });
    }
    let peers: Vec<_> = roster.peers().collect();
    let probed: Vec<Answer> = std::thread::scope(|scope| {
        let handles: Vec<_> = peers
            .iter()
            .map(|member| {
                let roster = &roster;
                scope.spawn(move || {
                    let name = member.device.name.clone();
                    let description = if member.device.online {
                        probe(env, roster, &name).map_err(|failure| failure.message)
                    } else {
                        Err("offline".to_string())
                    };
                    Answer { node: name, host: transport::host(member), this_node: false, description }
                })
            })
            .collect();
        handles.into_iter().map(|handle| handle.join().expect("probe thread")).collect()
    });
    answers.extend(probed);
    let (offers, unreachable) = capability::offers(wanted, answers);
    if has(&words, "--json") {
        println!("{}", json(&serde_json::json!({ "capability": wanted, "offers": offers, "unreachable": unreachable }))?);
    } else if has(&words, "--tsv") {
        for offer in &offers {
            println!("{}", tsv_line(offer));
        }
        for (node, reason) in &unreachable {
            eprintln!("dojo-mesh: could not ask {node}: {reason}");
        }
    } else {
        for offer in &offers {
            let attributes: Vec<String> = offer.attributes.iter().map(|(key, value)| format!("{key}={value}")).collect();
            println!("{wanted} offered by {} ({}){}", offer.node, offer.host, if attributes.is_empty() { String::new() } else { format!(": {}", attributes.join(" ")) });
        }
        for (node, reason) in &unreachable {
            eprintln!("dojo-mesh: could not ask {node}: {reason}");
        }
        if offers.is_empty() {
            println!("No node offers {wanted}.");
        }
    }
    Ok(if offers.is_empty() { 1 } else { 0 })
}

/// One offer as a tab-separated line for shell scripts. A value holding a tab or line break could
/// not be told apart from the separators, so it is left out rather than escaped.
fn tsv_line(offer: &capability::Offer) -> String {
    let plain = |text: &str| !text.contains(['\t', '\n', '\r']);
    let mut fields = vec![offer.node.clone(), offer.host.clone()];
    fields.extend(offer.attributes.iter().filter(|(key, value)| plain(key) && plain(value)).map(|(key, value)| format!("{key}={value}")));
    fields.join("\t")
}

// ---- copying files --------------------------------------------------------------------------

/// `peer:path` names a file on a peer. A path starting with `/` or `./`, or with a slash before
/// any colon, is local.
pub fn split_remote(arg: &str) -> Option<(&str, &str)> {
    let (peer, path) = arg.split_once(':')?;
    (!peer.is_empty() && !peer.contains('/') && !path.is_empty()).then_some((peer, path))
}

const PUSH_SCRIPT: &str = r#"set -e
dest=$1
case $dest in */) dest=$dest$2;; esac
if [ -d "$dest" ]; then dest=$dest/$2; fi
tmp=$(mktemp "$dest.XXXXXX")
trap 'rm -f "$tmp"' EXIT
cat > "$tmp"
chmod "$3" "$tmp"
mv -f "$tmp" "$dest""#;

fn copy(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let (words, _) = split(rest, &[], &[])?;
    let [source, destination] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, format!("cp takes a source and a destination\n{USAGE}"))) };
    let config = load_config(env)?;
    let roster = load_roster(env, &config);
    match (split_remote(source), split_remote(destination)) {
        (None, Some((peer, path))) => push(env, &roster, source, peer, path),
        (Some((peer, path)), None) => pull(env, &roster, peer, path, destination),
        _ => Err(fail(USAGE_ERROR, "exactly one side of cp must be PEER:PATH (start a local path with ./ if it contains a colon)")),
    }
}

fn base_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

fn push(env: &Env, roster: &Roster, source: &str, peer: &str, path: &str) -> Result<u8, Failure> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = std::fs::metadata(source).map_err(|error| fail(FAILED, format!("{source}: {error}")))?;
    if !metadata.is_file() {
        return Err(fail(FAILED, format!("{source} is not a regular file; cp sends one file at a time")));
    }
    let file = std::fs::File::open(source).map_err(|error| fail(FAILED, format!("{source}: {error}")))?;
    let mode = format!("{:04o}", metadata.permissions().mode() & 0o777);
    let argv: Vec<String> = ["sh", "-c", PUSH_SCRIPT, "sh", path, base_name(source), &mode].iter().map(|part| part.to_string()).collect();
    let done = call_peer(env, roster, peer, "cp", &format!("push {source} -> {peer}:{path}"), &argv, Input::File(file), Output::Inherit, None)?;
    outcome(peer, &done.ended)
}

fn pull(env: &Env, roster: &Roster, peer: &str, path: &str, destination: &str) -> Result<u8, Failure> {
    let mut target = PathBuf::from(destination);
    if target.is_dir() {
        target.push(base_name(path));
    }
    let directory = target.parent().filter(|parent| !parent.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let staging = directory.join(format!(".{}.dojo-mesh-{}", target.file_name().and_then(|n| n.to_str()).unwrap_or("file"), std::process::id()));
    let file = std::fs::File::create(&staging).map_err(|error| fail(FAILED, format!("{}: {error}", staging.display())))?;
    let argv: Vec<String> = ["cat", "--", path].iter().map(|part| part.to_string()).collect();
    let result = call_peer(env, roster, peer, "cp", &format!("pull {peer}:{path} -> {}", target.display()), &argv, Input::Null, Output::File(file), None)
        .and_then(|done| outcome(peer, &done.ended));
    match result {
        Ok(code) => {
            std::fs::rename(&staging, &target).map_err(|error| {
                let _ = std::fs::remove_file(&staging);
                fail(FAILED, format!("{}: {error}", target.display()))
            })?;
            Ok(code)
        }
        Err(failure) => {
            let _ = std::fs::remove_file(&staging);
            Err(failure)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peer_paths_are_told_from_local_paths() {
        assert_eq!(split_remote("beta:/etc/hostname"), Some(("beta", "/etc/hostname")));
        assert_eq!(split_remote("beta:notes.txt"), Some(("beta", "notes.txt")));
        assert_eq!(split_remote("./odd:name"), None);
        assert_eq!(split_remote("/abs/odd:name"), None);
        assert_eq!(split_remote("plain.txt"), None);
        assert_eq!(split_remote(":x"), None);
        assert_eq!(split_remote("beta:"), None);
    }

    #[test]
    fn tsv_lines_drop_values_that_would_break_the_columns() {
        let offer = capability::Offer {
            node: "beta".into(),
            host: "beta.example.ts.net".into(),
            this_node: false,
            attributes: [("port".to_string(), "5000".to_string()), ("bad".to_string(), "a\tb".to_string())].into(),
        };
        assert_eq!(tsv_line(&offer), "beta\tbeta.example.ts.net\tport=5000");
    }

    #[test]
    fn base_names_ignore_trailing_slashes() {
        assert_eq!(base_name("/a/b/c.txt"), "c.txt");
        assert_eq!(base_name("c.txt"), "c.txt");
        assert_eq!(base_name("/a/b/"), "b");
    }
}
