//! The commands that make a mesh of the signed-roster kind: `init`, `join`, `approve`, `renew`, `rm`,
//! `leave` and `trust`. A new node describes itself (`join`), the operator compares fingerprints and
//! signs (`approve`), and each node decides alone which signing keys it believes (`trust`).

use crate::cli::{self, Env, FAILED, Failure, NEEDS_HUMAN, UNAVAILABLE, USAGE_ERROR, fail, has, split};
use crate::config::{MeshConfig, MethodChoice, RosterPaths, roster_paths};
use crate::doctor::{Finding, Severity};
use crate::enroll::{self, JoinRequest, Signing};
use crate::signed::{self, Statement};
use chrono::{Duration, Utc};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const DEFAULT_DAYS: i64 = 90;
const MAX_DAYS: i64 = 366;

pub(crate) struct Ctx {
    pub(crate) config: MeshConfig,
    pub(crate) paths: RosterPaths,
}

pub(crate) fn context(env: &Env) -> Result<Ctx, Failure> {
    let config = cli::load_config(env)?;
    if config.method != MethodChoice::Roster {
        return Err(fail(USAGE_ERROR, "this command needs method = \"roster\" in the mesh configuration"));
    }
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let paths = roster_paths(&config, var("XDG_DATA_HOME"), var("XDG_CONFIG_HOME"), env.home.clone()).ok_or_else(|| fail(USAGE_ERROR, "there is nowhere to keep the mesh's files (HOME is not set)"))?;
    Ok(Ctx { config, paths })
}

impl Ctx {
    fn signing(&self) -> Signing {
        Signing { keys: self.config.signing_keys.clone(), agent: self.paths.agent.clone() }
    }
}

pub(crate) fn node_name(env: &Env, config: &MeshConfig) -> String {
    config.node_name.clone().unwrap_or_else(|| env.hostname.trim().to_ascii_lowercase())
}

pub(crate) fn value(values: &[(String, String)], flag: &str) -> Option<String> {
    values.iter().rev().find(|(name, _)| name == flag).map(|(_, value)| value.clone())
}

pub(crate) fn read_key(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let line = text.lines().map(str::trim).find(|line| !line.is_empty() && !line.starts_with('#')).ok_or_else(|| format!("{} holds no key", path.display()))?;
    signed::normalize_key(line).map_err(|error| format!("{}: {error}", path.display()))
}

pub(crate) fn write_private(path: &Path, text: &str, mode: u32) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(parent).map_err(|error| fail(FAILED, format!("{}: {error}", parent.display())))?;
    }
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(mode).open(path).map_err(|error| fail(FAILED, format!("{}: {error}", path.display())))?;
    std::io::Write::write_all(&mut file, text.as_bytes()).map_err(|error| fail(FAILED, format!("{}: {error}", path.display())))
}

fn print_fingerprints(request_name: &str, node: &str, input: Option<&str>, host: Option<&str>) -> Result<(), Failure> {
    let print = |what: &str, key: &str| -> Result<(), Failure> {
        println!("  {what:<10} {}", enroll::fingerprint(key).map_err(|message| fail(FAILED, message))?);
        Ok(())
    };
    println!("{request_name}:");
    print("node key", node)?;
    if let Some(input) = input {
        print("input key", input)?;
    }
    match host {
        Some(host) => print("host key", host),
        None => Ok(()),
    }
}

/// Makes the roster's `git` remote aware of a change, when the roster is a repository. Failing to
/// publish is reported and never undoes the signed change.
fn publish(dir: &Path, message: &str) -> String {
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(dir).args(args).stdin(Stdio::null()).output();
    if !git(&["rev-parse", "--is-inside-work-tree"]).is_ok_and(|out| out.status.success()) {
        return "the roster is not a git repository: members that are online are sent it directly, and the others take it with `dojo-mesh roster sync`".into();
    }
    let _ = git(&["add", "-A", "--", "."]);
    match git(&["commit", "-q", "-m", message, "--", "."]) {
        Ok(out) if out.status.success() => {}
        Ok(out) => return format!("could not commit the roster: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(error) => return format!("could not run git: {error}"),
    }
    if !git(&["remote"]).is_ok_and(|out| !out.stdout.is_empty()) {
        return "committed to the roster; it has no remote, so the other nodes must be given it".into();
    }
    match git(&["push", "-q"]) {
        Ok(out) if out.status.success() => "committed and pushed; other nodes get it from `dojo-mesh roster sync`".into(),
        Ok(out) => format!("committed, but the push failed: {}", String::from_utf8_lossy(&out.stderr).trim()),
        Err(error) => format!("committed, but git could not run: {error}"),
    }
}

/// Lets this node's own `authorized_keys` follow the roster after it changed. A failure here is a
/// warning: the signed change stands.
fn follow(env: &Env, config: &MeshConfig) {
    if let Err(failure) = cli::authorize(env, config) {
        eprintln!("dojo-mesh: could not update authorized_keys: {}", failure.message);
    }
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    if let Some(paths) = roster_paths(config, var("XDG_DATA_HOME"), var("XDG_CONFIG_HOME"), env.home.clone()) {
        // A roster kept in git reaches the others through its remote (publish); otherwise hand it to the members.
        if !crate::openssh::has_remote(&paths.dir) {
            cli::spread(env, config, &paths.dir);
        }
    }
}

/// `dojo-mesh init`: gives this node its two keys. By default they are made in 1Password (through
/// its `op` command line) so the private halves never sit in a file; keys made anywhere else
/// (a YubiKey, another agent) are registered with `--custody existing`.
pub fn init(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &[], &["--custody", "--node-key", "--input-key", "--vault"])?;
    if !words.positional.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh init [--custody 1password|existing] [--vault NAME] [--node-key PUB --input-key PUB]"));
    }
    if ctx.paths.node_key.exists() || ctx.paths.input_key.exists() {
        return Err(fail(USAGE_ERROR, format!("this node already has keys ({} / {}); remove them first to start over", ctx.paths.node_key.display(), ctx.paths.input_key.display())));
    }
    let name = node_name(env, &ctx.config);
    signed::valid_name(&name).map_err(|reason| fail(USAGE_ERROR, format!("{reason}; set node-name in mesh.toml")))?;
    let (node, input) = match value(&values, "--custody").as_deref().unwrap_or("1password") {
        "1password" => {
            let vault = value(&values, "--vault");
            let node = onepassword_key(&format!("dojo-mesh node key ({name})"), vault.as_deref())?;
            let input = onepassword_key(&format!("dojo-mesh input key ({name})"), vault.as_deref())?;
            (node, Some(input))
        }
        "existing" => {
            let node = value(&values, "--node-key").ok_or_else(|| fail(USAGE_ERROR, "--custody existing needs --node-key PUBLIC_KEY_FILE"))?;
            let node = read_key(Path::new(&node)).map_err(|message| fail(USAGE_ERROR, message))?;
            let input = value(&values, "--input-key").map(|path| read_key(Path::new(&path))).transpose().map_err(|message| fail(USAGE_ERROR, message))?;
            (node, input)
        }
        other => return Err(fail(USAGE_ERROR, format!("custody {other:?} is not known; use 1password or existing"))),
    };
    write_private(&ctx.paths.node_key, &format!("{node} dojo-mesh:{name}\n"), 0o644)?;
    if let Some(input) = &input {
        write_private(&ctx.paths.input_key, &format!("{input} dojo-mesh:{name}:input\n"), 0o644)?;
    }
    print_fingerprints(&format!("{name} now has"), &node, input.as_deref(), None)?;
    println!("Next: `dojo-mesh join` makes the request to send to the operator.");
    Ok(0)
}

/// Creates an SSH key in 1Password and returns its public half.
pub(crate) fn onepassword_key(title: &str, vault: Option<&str>) -> Result<String, Failure> {
    let mut command = Command::new("op");
    command.args(["item", "create", "--category", "SSH Key", "--ssh-generate-key", "Ed25519", "--title", title, "--format", "json"]);
    if let Some(vault) = vault {
        command.args(["--vault", vault]);
    }
    let output = command.stdin(Stdio::null()).output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            fail(UNAVAILABLE, "1Password's command line (`op`) is not installed; install it, or make the keys yourself and register them with --custody existing")
        } else {
            fail(UNAVAILABLE, format!("could not run op: {error}"))
        }
    })?;
    if !output.status.success() {
        let text = String::from_utf8_lossy(&output.stderr);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("op failed").to_string();
        let lower = line.to_ascii_lowercase();
        let code = if lower.contains("sign in") || lower.contains("signin") || lower.contains("not signed") || lower.contains("authoriz") { NEEDS_HUMAN } else { FAILED };
        return Err(fail(code, if code == NEEDS_HUMAN { format!("1Password needs you first: unlock it or run `op signin` ({line})") } else { format!("op could not create the key: {line}") }));
    }
    public_key_of(&String::from_utf8_lossy(&output.stdout)).map_err(|message| fail(FAILED, message))
}

/// The public key in `op item create --format json` output.
fn public_key_of(json: &str) -> Result<String, String> {
    let item: serde_json::Value = serde_json::from_str(json).map_err(|error| format!("op gave an item that cannot be read: {error}"))?;
    let key = item["fields"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|field| field["id"] == "public_key" || field["label"].as_str().is_some_and(|label| label.eq_ignore_ascii_case("public key")))
        .and_then(|field| field["value"].as_str())
        .ok_or("op's item has no public key")?;
    signed::normalize_key(key.lines().next().unwrap_or_default())
}

/// `dojo-mesh join`: writes this node's request to be enrolled and shows the fingerprints to compare.
pub fn join(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &[], &["--route", "--host-key-file", "--out"])?;
    if !words.positional.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh join [--route HOST[:PORT]]... [--host-key-file PATH] [--out FILE]"));
    }
    let name = node_name(env, &ctx.config);
    let node = read_key(&ctx.paths.node_key).map_err(|message| fail(USAGE_ERROR, format!("{message}; run dojo-mesh init first")))?;
    let input = if ctx.paths.input_key.exists() { Some(read_key(&ctx.paths.input_key).map_err(|message| fail(USAGE_ERROR, message))?) } else { None };
    let host_file = value(&values, "--host-key-file").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/etc/ssh/ssh_host_ed25519_key.pub"));
    let host = read_key(&host_file).map_err(|message| fail(UNAVAILABLE, format!("{message}; this machine needs an SSH server (see docs/mesh.md) before others can connect to it")))?;
    let mut routes = Vec::new();
    for (flag, text) in &values {
        if flag == "--route" {
            routes.push(enroll::parse_route(text).map_err(|message| fail(USAGE_ERROR, message))?);
        }
    }
    let mut request = JoinRequest { v: 2, name: name.clone(), node_key: node.clone(), input_key: input.clone(), host_key: host.clone(), routes, signature: String::new() };
    // The node key vouches for the whole request, so the operator can trust every field in it by
    // comparing that key's fingerprint alone. The key is in the agent (1Password), which may ask.
    request.signature = enroll::sign_bytes(&request.body(), &ctx.paths.node_key, enroll::JOIN_NAMESPACE, ctx.paths.agent.as_deref())
        .map_err(|message| fail(NEEDS_HUMAN, format!("the request could not be signed with this node's key ({message}); unlock 1Password and turn on its SSH agent, then try again")))?;
    let text = serde_json::to_string_pretty(&request).map_err(|error| fail(FAILED, error.to_string()))? + "\n";
    JoinRequest::parse(&text).and_then(|parsed| parsed.verify().map(|()| parsed)).map_err(|message| fail(FAILED, message))?;
    let out = value(&values, "--out").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("{name}.join.json")));
    write_private(&out, &text, 0o644)?;
    print_fingerprints(&format!("{name} asks to join; compare these with what the operator sees"), &node, input.as_deref(), Some(&host))?;
    println!("Request written to {}. Give it to the operator.", out.display());
    Ok(0)
}

pub(crate) fn device_file(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.paths.dir.join("devices").join(format!("{name}.json"))
}

/// The statement in a device file, as written (not verified: it is only read to decide what to do next).
fn read_device(path: &Path) -> Result<signed::DeviceEntry, String> {
    let text = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
    match serde_json::from_slice::<Statement>(&text) {
        Ok(Statement::Device(entry)) => Ok(entry),
        Ok(_) => Err(format!("{} is not a device entry", path.display())),
        Err(error) => Err(format!("{}: {error}", path.display())),
    }
}

fn days(values: &[(String, String)]) -> Result<i64, Failure> {
    match value(values, "--days") {
        None => Ok(DEFAULT_DAYS),
        Some(text) => match text.parse::<i64>() {
            Ok(days) if (1..=MAX_DAYS).contains(&days) => Ok(days),
            _ => Err(fail(USAGE_ERROR, format!("--days must be from 1 to {MAX_DAYS}"))),
        },
    }
}

fn revoked_keys(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir.join("revocations")) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| std::fs::read(entry.path()).ok())
        .filter_map(|bytes| match serde_json::from_slice::<Statement>(&bytes) {
            Ok(Statement::Revoke(revocation)) => signed::normalize_key(&revocation.key).ok(),
            _ => None,
        })
        .collect()
}

/// `dojo-mesh approve REQUEST`: signs a device entry for the node that sent it, once the operator
/// has compared its node key's fingerprint with the one the new node shows.
pub fn approve(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &["--replace"], &["--fingerprint", "--days"])?;
    let [file] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, "usage: dojo-mesh approve REQUEST.json [--fingerprint SHA256:…] [--days N] [--replace]")) };
    let text = std::fs::read_to_string(file).map_err(|error| fail(USAGE_ERROR, format!("{file}: {error}")))?;
    let request = JoinRequest::parse(&text).map_err(|message| fail(USAGE_ERROR, format!("{file}: {message}")))?;
    // Before anything is shown to compare: a request that its own node key did not sign, or that was
    // changed afterwards, is not worth comparing.
    request.verify().map_err(|message| fail(FAILED, format!("{file}: {message}. Do not approve it")))?;
    let days = days(&values)?;
    let node_fp = enroll::fingerprint(&request.node_key).map_err(|message| fail(FAILED, message))?;
    let Some(given) = value(&values, "--fingerprint") else {
        print_fingerprints(&format!("{} asks to join", request.name), &request.node_key, request.input_key.as_deref(), Some(&request.host_key))?;
        return Err(fail(NEEDS_HUMAN, format!("compare the node key's fingerprint with what {} shows, then run this again with --fingerprint {node_fp}", request.name)));
    };
    if given.trim() != node_fp {
        return Err(fail(FAILED, format!("the fingerprint does not match: the request's node key is {node_fp}, not {given}. Do not approve it")));
    }
    let path = device_file(&ctx, &request.name);
    if path.exists() {
        let known = read_device(&path).map(|entry| entry.node_key);
        if known.as_deref().ok() != Some(request.node_key.as_str()) && !has(&words, "--replace") {
            return Err(fail(FAILED, format!("{} is already in the roster with a different key; pass --replace if this really is the same machine re-enrolling", request.name)));
        }
    }
    if let Some(key) = [Some(&request.node_key), request.input_key.as_ref(), Some(&request.host_key)].into_iter().flatten().find(|key| revoked_keys(&ctx.paths.dir).contains(key)) {
        return Err(fail(FAILED, format!("a key in this request ({}) was revoked; the node needs new keys", enroll::fingerprint(key).unwrap_or_default())));
    }
    let entry = request.entry(Utc::now(), days);
    enroll::write_signed(&path, &enroll::device_json(&entry), &ctx.signing()).map_err(|message| fail(code_for(&message), message))?;
    println!("approved {} until {}", request.name, entry.expires.format("%Y-%m-%d"));
    println!("{}", publish(&ctx.paths.dir, &format!("Enroll {}", request.name)));
    follow(env, &ctx.config);
    Ok(0)
}

/// A signing failure that a person can fix (a key to touch or unlock) is exit 3.
fn code_for(message: &str) -> u8 {
    if message.contains("no signing key is set") { USAGE_ERROR } else { NEEDS_HUMAN }
}

/// `dojo-mesh renew NAME [--days N]`: the same entry, signed again for a fresh period.
pub fn renew(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &[], &["--days"])?;
    let [name] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, "usage: dojo-mesh renew NAME [--days N]")) };
    let days = days(&values)?;
    let path = device_file(&ctx, name);
    let mut entry = read_device(&path).map_err(|message| fail(FAILED, message))?;
    if revoked_keys(&ctx.paths.dir).contains(&entry.node_key) {
        return Err(fail(FAILED, format!("{name}'s key was revoked; it must be enrolled again with new keys")));
    }
    entry.issued = Utc::now();
    entry.expires = entry.issued + Duration::days(days);
    enroll::write_signed(&path, &enroll::device_json(&entry), &ctx.signing()).map_err(|message| fail(code_for(&message), message))?;
    println!("renewed {name} until {}", entry.expires.format("%Y-%m-%d"));
    println!("{}", publish(&ctx.paths.dir, &format!("Renew {name}")));
    follow(env, &ctx.config);
    Ok(0)
}

/// `dojo-mesh rm NAME`: withdraws a node. Its entry is removed and its keys are revoked, so a copy of
/// the entry somewhere else stops counting too.
pub fn rm(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &[], &["--reason"])?;
    let [name] = words.positional.as_slice() else { return Err(fail(USAGE_ERROR, "usage: dojo-mesh rm NAME [--reason TEXT]")) };
    let path = device_file(&ctx, name);
    let entry = read_device(&path).map_err(|message| fail(FAILED, format!("{message}; there is no such node in the roster")))?;
    let reason = value(&values, "--reason");
    let now = Utc::now();
    let signing = ctx.signing();
    for (kind, key) in [("node", Some(&entry.node_key)), ("input", entry.input_key.as_ref()), ("host", Some(&entry.host_key))].into_iter().filter_map(|(kind, key)| key.map(|key| (kind, key))) {
        let fp = enroll::fingerprint(key).map_err(|message| fail(FAILED, message))?;
        let tail: String = fp.trim_start_matches("SHA256:").chars().filter(char::is_ascii_alphanumeric).take(10).collect();
        let revocation = ctx.paths.dir.join("revocations").join(format!("{name}-{kind}-{tail}.json"));
        enroll::write_signed(&revocation, &enroll::revocation_json(key, reason.as_deref(), now), &signing).map_err(|message| fail(code_for(&message), message))?;
    }
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}.sig", path.display()));
    println!("removed {name} and revoked its keys");
    println!("{}", publish(&ctx.paths.dir, &format!("Remove {name}")));
    follow(env, &ctx.config);
    Ok(0)
}

/// `dojo-mesh leave`: this node stops letting the mesh in. Its keys, roster and trusted signers stay
/// where they are, so rejoining is `roster authorize`.
pub fn leave(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    if !rest.is_empty() {
        return Err(fail(USAGE_ERROR, "usage: dojo-mesh leave"));
    }
    let home = env.home.clone().ok_or_else(|| fail(USAGE_ERROR, "HOME is not set"))?;
    let path = home.join(".ssh/authorized_keys");
    let changed = crate::authorize::apply(&path, &[]).map_err(|message| fail(FAILED, message))?;
    let _ = std::fs::remove_dir_all(&ctx.paths.known_hosts);
    println!("{}", if changed { format!("removed the dojo-mesh block from {}", path.display()) } else { "there was no dojo-mesh block to remove".to_string() });
    println!("To stop for good, also remove method = \"roster\" from mesh.toml; the other nodes drop this one with `dojo-mesh rm {}`.", node_name(env, &ctx.config));
    Ok(0)
}

/// `dojo-mesh trust list | add KEY --fingerprint FP | remove FP`: the signing keys this node believes,
/// in its own file. Adding one takes the fingerprint the person compared, so a key is never trusted
/// on the strength of where it came from.
pub fn trust(env: &Env, rest: &[String]) -> Result<u8, Failure> {
    let ctx = context(env)?;
    let (words, values) = split(rest, &[], &["--fingerprint"])?;
    let signers_text = std::fs::read_to_string(&ctx.paths.signers).unwrap_or_default();
    let current = enroll::signers(&signers_text);
    match words.positional.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] | ["list"] => {
            if current.is_empty() {
                println!("No signing keys are trusted ({}). Add one with `dojo-mesh trust add KEY --fingerprint SHA256:…`.", ctx.paths.signers.display());
            }
            for key in &current {
                println!("{}  {}  {}", enroll::fingerprint(key).map_err(|message| fail(FAILED, message))?, key.split_whitespace().next().unwrap_or(""), enroll::held_as(key));
            }
            Ok(0)
        }
        ["add", source] => {
            let given = value(&values, "--fingerprint").ok_or_else(|| fail(USAGE_ERROR, "trust add needs --fingerprint SHA256:… of the key, as read off the device that holds it"))?;
            let key = if Path::new(source).exists() { read_key(Path::new(source)).map_err(|message| fail(USAGE_ERROR, message))? } else { signed::normalize_key(source).map_err(|message| fail(USAGE_ERROR, message))? };
            let fp = enroll::fingerprint(&key).map_err(|message| fail(FAILED, message))?;
            if given.trim() != fp {
                return Err(fail(FAILED, format!("the fingerprint does not match: that key is {fp}, not {given}. It was not trusted")));
            }
            if current.contains(&key) {
                println!("{fp} is already trusted");
                return Ok(0);
            }
            let mut text = signers_text;
            if !text.is_empty() && !text.ends_with('\n') {
                text.push('\n');
            }
            text.push_str(&enroll::signer_line(&key));
            text.push('\n');
            write_private(&ctx.paths.signers, &text, 0o600)?;
            println!("now trusting {fp} ({})", enroll::held_as(&key));
            Ok(0)
        }
        ["remove", wanted] => {
            let mut kept = Vec::new();
            let mut removed = false;
            for line in signers_text.lines() {
                let keys = enroll::signers(line);
                let matches = match keys.first() {
                    Some(key) => enroll::fingerprint(key).is_ok_and(|fp| fp == *wanted),
                    None => false,
                };
                removed |= matches;
                if !matches {
                    kept.push(line);
                }
            }
            if !removed {
                return Err(fail(FAILED, format!("no trusted signing key has the fingerprint {wanted}")));
            }
            write_private(&ctx.paths.signers, &(kept.join("\n") + if kept.is_empty() { "" } else { "\n" }), 0o600)?;
            println!("no longer trusting {wanted}");
            Ok(0)
        }
        _ => Err(fail(USAGE_ERROR, "usage: dojo-mesh trust [list | add KEY --fingerprint SHA256:… | remove SHA256:…]")),
    }
}

/// Findings about how the signing keys are held, for `doctor`. The roster's own problems come from
/// the roster; this is about the trust a node was set up with.
pub fn custody_findings(env: &Env, config: &MeshConfig) -> Vec<Finding> {
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let Some(paths) = roster_paths(config, var("XDG_DATA_HOME"), var("XDG_CONFIG_HOME"), env.home.clone()) else { return Vec::new() };
    let keys = enroll::signers(&std::fs::read_to_string(&paths.signers).unwrap_or_default());
    if keys.is_empty() || keys.iter().any(|key| key.starts_with("sk-")) {
        return Vec::new();
    }
    vec![Finding {
        severity: Severity::Warning,
        device: None,
        message: "Every trusted signing key is held in software (a file or an agent). Anyone who can use this computer while it is unlocked can sign roster entries if the agent remembers approvals; a hardware security key would need a touch each time.".into(),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_key_is_found_in_the_item_op_returns() {
        let item = r#"{"id":"x","fields":[{"id":"private_key","value":"-----BEGIN"},{"id":"public_key","label":"public key","value":"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}]}"#;
        assert_eq!(public_key_of(item).unwrap(), "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA");
        assert!(public_key_of(r#"{"fields":[]}"#).is_err());
        assert!(public_key_of("not json").is_err());
        let by_label = r#"{"fields":[{"label":"Public Key","value":"ssh-ed25519 AAAA"}]}"#;
        assert_eq!(public_key_of(by_label).unwrap(), "ssh-ed25519 AAAA");
    }

    #[test]
    fn a_signing_failure_a_person_can_fix_is_exit_3_and_a_missing_setting_is_a_usage_error() {
        assert_eq!(code_for("no signing key could sign (/k.pub: communication with agent failed)"), NEEDS_HUMAN);
        assert_eq!(code_for("no signing key is set: list the keys"), USAGE_ERROR);
    }

    #[test]
    fn the_days_are_bounded() {
        assert_eq!(days(&[]).unwrap(), 90);
        assert_eq!(days(&[("--days".into(), "30".into())]).unwrap(), 30);
        for bad in ["0", "400", "x", "-1"] {
            assert!(days(&[("--days".into(), bad.into())]).is_err(), "{bad}");
        }
    }
}
