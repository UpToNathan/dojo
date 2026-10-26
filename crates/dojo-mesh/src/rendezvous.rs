//! Where two machines leave messages for each other while one joins the other's mesh. The operator
//! posts an **invite** (its name, the signing keys it trusts, an expiry); the new machine finds it,
//! posts its signed join request into the same item; the operator approves and posts the answer (the
//! roster); the new machine installs it. Nothing is typed or carried by hand: the carrier is a place
//! both machines can read, and for a person with 1Password that is a vault item, which is also what
//! makes the exchange trustworthy: only the person's own account can write there, as it is already
//! what holds the nodes' keys.
//!
//! The carrier is an interface so another can be added (a shared directory is here for tests and for
//! people without 1Password; a relay is the likely next one). The protocol on top of it is the same.

use crate::config::RendezvousChoice;
use crate::enroll::JoinRequest;
use chrono::{DateTime, Duration, Utc};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// What marks an item as one of ours, in 1Password and in a directory alike.
pub const TAG: &str = "dojo-mesh-invite";
/// How long an invite lasts unless asked otherwise.
pub const INVITE_MINUTES: i64 = 15;
/// A request or an answer is a few KiB; this is the most a field may hold.
const MAX_FIELD: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: String,
    pub fields: BTreeMap<String, String>,
}

/// A place to leave short messages as items made of text fields.
pub trait Rendezvous {
    fn create(&self, fields: &BTreeMap<String, String>) -> Result<Item, String>;
    fn list(&self) -> Result<Vec<Item>, String>;
    fn get(&self, id: &str) -> Result<Item, String>;
    fn update(&self, id: &str, fields: &BTreeMap<String, String>) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// The carrier a configuration names; 1Password's default vault when it names none.
pub fn open(choice: &Option<RendezvousChoice>) -> Box<dyn Rendezvous> {
    match choice {
        Some(RendezvousChoice::Directory(dir)) => Box::new(Directory { dir: dir.clone() }),
        Some(RendezvousChoice::OnePassword { vault }) => Box::new(OnePassword { vault: vault.clone(), program: PathBuf::from("op") }),
        None => Box::new(OnePassword { vault: None, program: PathBuf::from("op") }),
    }
}

// ---- 1Password -------------------------------------------------------------------------------

pub struct OnePassword {
    pub vault: Option<String>,
    /// The `op` command line; a test points it at a fake.
    pub program: PathBuf,
}

impl OnePassword {
    fn run(&self, args: &[String]) -> Result<Vec<u8>, String> {
        let output = Command::new(&self.program).args(args).stdin(Stdio::null()).output().map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound { "1Password's command line (op) is not installed".to_string() } else { format!("could not run op: {error}") }
        })?;
        if output.status.success() {
            return Ok(output.stdout);
        }
        let text = String::from_utf8_lossy(&output.stderr);
        let line = text.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("op failed").to_string();
        let lower = line.to_ascii_lowercase();
        Err(if lower.contains("sign in") || lower.contains("signin") || lower.contains("not signed") || lower.contains("authoriz") { format!("1Password needs you first: unlock it ({line})") } else { line })
    }

    fn vault_args(&self) -> Vec<String> {
        self.vault.iter().flat_map(|vault| ["--vault".to_string(), vault.clone()]).collect()
    }

    /// `name[text]=value` for each field: how `op` takes custom fields.
    fn assignments(fields: &BTreeMap<String, String>) -> Vec<String> {
        fields.iter().map(|(name, value)| format!("{name}[text]={value}")).collect()
    }

    fn parse_item(bytes: &[u8]) -> Result<Item, String> {
        let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| format!("op gave an item that cannot be read: {error}"))?;
        let id = value["id"].as_str().ok_or("op's item has no id")?.to_string();
        let mut fields = BTreeMap::new();
        for field in value["fields"].as_array().into_iter().flatten() {
            if let (Some(label), Some(text)) = (field["label"].as_str(), field["value"].as_str()) {
                fields.insert(label.to_string(), text.to_string());
            }
        }
        Ok(Item { id, fields })
    }
}

impl Rendezvous for OnePassword {
    fn create(&self, fields: &BTreeMap<String, String>) -> Result<Item, String> {
        let mut args: Vec<String> = ["item", "create", "--category", "Secure Note", "--title", "Dojo mesh invite", "--tags", TAG, "--format", "json"].iter().map(|part| part.to_string()).collect();
        args.extend(self.vault_args());
        args.extend(Self::assignments(fields));
        Self::parse_item(&self.run(&args)?)
    }

    fn list(&self) -> Result<Vec<Item>, String> {
        let mut args: Vec<String> = ["item", "list", "--tags", TAG, "--format", "json"].iter().map(|part| part.to_string()).collect();
        args.extend(self.vault_args());
        let listed: serde_json::Value = serde_json::from_slice(&self.run(&args)?).map_err(|error| format!("op gave a list that cannot be read: {error}"))?;
        listed.as_array().into_iter().flatten().filter_map(|entry| entry["id"].as_str()).map(|id| self.get(id)).collect()
    }

    fn get(&self, id: &str) -> Result<Item, String> {
        Self::parse_item(&self.run(&["item".into(), "get".into(), id.into(), "--format".into(), "json".into()])?)
    }

    fn update(&self, id: &str, fields: &BTreeMap<String, String>) -> Result<(), String> {
        let mut args = vec!["item".to_string(), "edit".into(), id.into()];
        args.extend(Self::assignments(fields));
        self.run(&args).map(|_| ())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.run(&["item".into(), "delete".into(), id.into()]).map(|_| ())
    }
}

// ---- A directory -----------------------------------------------------------------------------

pub struct Directory {
    pub dir: PathBuf,
}

impl Directory {
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(format!("{id:?} is not an invite id"));
        }
        Ok(self.dir.join(format!("{id}.json")))
    }

    fn write(&self, id: &str, fields: &BTreeMap<String, String>) -> Result<(), String> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&self.dir).map_err(|error| format!("{}: {error}", self.dir.display()))?;
        let path = self.path(id)?;
        let temporary = self.dir.join(format!(".{id}.tmp"));
        let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&temporary).map_err(|error| format!("{}: {error}", temporary.display()))?;
        std::io::Write::write_all(&mut file, serde_json::to_string(fields).map_err(|error| error.to_string())?.as_bytes()).map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, &path).map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// Sixteen random bytes as hex, from the system's source.
fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)).map_err(|error| format!("no randomness: {error}"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl Rendezvous for Directory {
    fn create(&self, fields: &BTreeMap<String, String>) -> Result<Item, String> {
        let id = random_id()?;
        self.write(&id, fields)?;
        Ok(Item { id, fields: fields.clone() })
    }

    fn list(&self) -> Result<Vec<Item>, String> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("{}: {error}", self.dir.display())),
        };
        Ok(entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let id = path.file_name()?.to_str()?.strip_suffix(".json")?.to_string();
                if id.starts_with('.') || !entry.file_type().ok()?.is_file() || entry.metadata().ok()?.len() > (MAX_FIELD as u64) * 4 {
                    return None;
                }
                Some(Item { id, fields: serde_json::from_slice(&std::fs::read(&path).ok()?).ok()? })
            })
            .collect())
    }

    fn get(&self, id: &str) -> Result<Item, String> {
        let path = self.path(id)?;
        let bytes = std::fs::read(&path).map_err(|error| format!("that invite is gone ({}: {error})", path.display()))?;
        Ok(Item { id: id.to_string(), fields: serde_json::from_slice(&bytes).map_err(|error| format!("{}: {error}", path.display()))? })
    }

    fn update(&self, id: &str, fields: &BTreeMap<String, String>) -> Result<(), String> {
        let mut item = self.get(id)?;
        item.fields.extend(fields.clone());
        self.write(id, &item.fields)
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        std::fs::remove_file(self.path(id)?).map_err(|error| error.to_string())
    }
}

// ---- The exchange ----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Open,
    Requested,
    Approved,
    Denied,
}

impl Status {
    fn word(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Requested => "requested",
            Self::Approved => "approved",
            Self::Denied => "denied",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [Self::Open, Self::Requested, Self::Approved, Self::Denied].into_iter().find(|status| status.word() == word)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub id: String,
    pub operator: String,
    /// The signing keys the operator trusts, `type base64`: what the new machine will trust.
    pub signers: Vec<String>,
    pub expires: DateTime<Utc>,
    pub status: Status,
}

fn field(item: &Item, name: &str) -> Option<String> {
    item.fields.get(name).filter(|value| value.len() <= MAX_FIELD).cloned()
}

fn invite_of(item: &Item) -> Option<Invite> {
    if item.fields.get("kind").map(String::as_str) != Some(TAG) {
        return None;
    }
    Some(Invite {
        id: item.id.clone(),
        operator: field(item, "operator")?,
        signers: field(item, "signers")?.lines().filter(|line| !line.trim().is_empty()).map(str::to_string).collect(),
        expires: DateTime::parse_from_rfc3339(&field(item, "expires")?).ok()?.with_timezone(&Utc),
        status: Status::from_word(&field(item, "status")?)?,
    })
}

/// Posts an invite and returns its id.
pub fn post_invite(carrier: &dyn Rendezvous, operator: &str, signers: &[String], now: DateTime<Utc>, minutes: i64) -> Result<Invite, String> {
    if signers.is_empty() {
        return Err("this node trusts no signing key yet, so there is nothing to invite anyone to".into());
    }
    let fields: BTreeMap<String, String> = [
        ("kind", TAG.to_string()),
        ("operator", operator.to_string()),
        ("signers", signers.join("\n")),
        ("expires", (now + Duration::minutes(minutes)).to_rfc3339()),
        ("status", Status::Open.word().to_string()),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_string(), value))
    .collect();
    let item = carrier.create(&fields)?;
    invite_of(&item).ok_or_else(|| "the invite could not be read back".to_string())
}

/// The invites that have not expired, whatever their state.
pub fn invites(carrier: &dyn Rendezvous, now: DateTime<Utc>) -> Result<Vec<Invite>, String> {
    let mut found: Vec<Invite> = carrier.list()?.iter().filter_map(invite_of).filter(|invite| invite.expires > now).collect();
    found.sort_by(|a, b| b.expires.cmp(&a.expires));
    Ok(found)
}

/// Deletes the invites that expired. How many went.
pub fn sweep(carrier: &dyn Rendezvous, now: DateTime<Utc>) -> Result<usize, String> {
    let mut gone = 0;
    for item in carrier.list()? {
        if let Some(invite) = invite_of(&item) {
            if invite.expires <= now && carrier.delete(&item.id).is_ok() {
                gone += 1;
            }
        }
    }
    Ok(gone)
}

/// The new machine's side: leaves its request in an open invite. One request per invite.
pub fn post_request(carrier: &dyn Rendezvous, id: &str, request: &JoinRequest, now: DateTime<Utc>) -> Result<(), String> {
    let item = carrier.get(id)?;
    let invite = invite_of(&item).ok_or("that is not an invite")?;
    if invite.expires <= now {
        return Err("that invite has expired".into());
    }
    if invite.status != Status::Open {
        return Err("that invite already has a request".into());
    }
    let text = serde_json::to_string(request).map_err(|error| error.to_string())?;
    let fields = [("request", text), ("status", Status::Requested.word().to_string())].into_iter().map(|(name, value)| (name.to_string(), value)).collect();
    carrier.update(id, &fields)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub invite: Invite,
    pub request: JoinRequest,
}

/// The requests waiting for an answer: in a live invite, parsing, and signed by their own node key.
/// The count is of those that were not taken.
pub fn pending(carrier: &dyn Rendezvous, now: DateTime<Utc>) -> Result<(Vec<Pending>, usize), String> {
    let (mut found, mut skipped) = (Vec::new(), 0);
    for item in carrier.list()? {
        let Some(invite) = invite_of(&item) else { continue };
        if invite.expires <= now || invite.status != Status::Requested {
            continue;
        }
        match field(&item, "request").and_then(|text| JoinRequest::parse(&text).ok()).filter(|request| request.verify().is_ok()) {
            Some(request) => found.push(Pending { invite, request }),
            None => skipped += 1,
        }
    }
    Ok((found, skipped))
}

/// The operator's answer: the roster, as `roster export` makes it.
pub fn post_response(carrier: &dyn Rendezvous, id: &str, bundle: &str) -> Result<(), String> {
    if bundle.len() > MAX_FIELD {
        return Err("the roster is too large to send this way".into());
    }
    let fields = [("response", bundle.to_string()), ("status", Status::Approved.word().to_string())].into_iter().map(|(name, value)| (name.to_string(), value)).collect();
    carrier.update(id, &fields)
}

pub fn deny(carrier: &dyn Rendezvous, id: &str, reason: &str) -> Result<(), String> {
    let fields = [("reason", reason.to_string()), ("status", Status::Denied.word().to_string())].into_iter().map(|(name, value)| (name.to_string(), value)).collect();
    carrier.update(id, &fields)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    Waiting,
    Approved { bundle: String, signers: Vec<String> },
    Denied(String),
    /// The invite is gone or expired.
    Gone,
}

/// What the new machine hears back.
pub fn answer(carrier: &dyn Rendezvous, id: &str, now: DateTime<Utc>) -> Result<Answer, String> {
    let Ok(item) = carrier.get(id) else { return Ok(Answer::Gone) };
    let Some(invite) = invite_of(&item) else { return Ok(Answer::Gone) };
    Ok(match invite.status {
        Status::Approved => Answer::Approved { bundle: field(&item, "response").ok_or("the answer is missing")?, signers: invite.signers },
        Status::Denied => Answer::Denied(field(&item, "reason").unwrap_or_else(|| "the request was turned down".into())),
        _ if invite.expires <= now => Answer::Gone,
        _ => Answer::Waiting,
    })
}

/// Where a directory carrier keeps things, for a configuration that names one.
pub fn directory_of(choice: &Option<RendezvousChoice>) -> Option<&Path> {
    match choice {
        Some(RendezvousChoice::Directory(dir)) => Some(dir),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::enroll;
    use crate::signed::RouteSpec;
    use std::os::unix::fs::PermissionsExt;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-rendezvous-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A request signed by a fresh key pair, as `dojo-mesh join` makes one.
    fn request(folder: &Path, name: &str) -> JoinRequest {
        // A fresh key each time, even for the same name: ssh-keygen would ask before overwriting.
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let private = folder.join(format!("{name}-key-{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        assert!(Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&private).stdin(Stdio::null()).status().unwrap().success());
        let public = std::fs::read_to_string(format!("{}.pub", private.display())).unwrap().split_whitespace().take(2).collect::<Vec<_>>().join(" ");
        let mut request = JoinRequest { v: 2, name: name.into(), node_key: public.clone(), input_key: None, host_key: public, routes: vec![RouteSpec { host: "192.0.2.9".into(), port: Some(2222) }], signature: String::new() };
        request.signature = enroll::sign_bytes(&request.body(), Path::new(&format!("{}.pub", private.display())), enroll::JOIN_NAMESPACE, None).unwrap();
        request
    }

    const SIGNER: &str = "sk-ssh-ed25519@openssh.com AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29t";

    fn at(minutes: i64) -> DateTime<Utc> {
        Utc::now() + Duration::minutes(minutes)
    }

    /// The whole exchange, on any carrier.
    fn exchange(carrier: &dyn Rendezvous, folder: &Path) {
        let invite = post_invite(carrier, "orchard", &[SIGNER.to_string()], Utc::now(), 15).unwrap();
        assert_eq!((invite.operator.as_str(), invite.status), ("orchard", Status::Open));
        assert_eq!(invite.signers, [SIGNER]);
        // The new machine finds it, and leaves its request.
        let found = invites(carrier, Utc::now()).unwrap();
        assert_eq!(found.iter().map(|i| i.id.clone()).collect::<Vec<_>>(), [invite.id.clone()]);
        let asking = request(folder, "studio");
        post_request(carrier, &invite.id, &asking, Utc::now()).unwrap();
        assert_eq!(answer(carrier, &invite.id, Utc::now()).unwrap(), Answer::Waiting);
        // The operator sees it, once, with its signature good.
        let (waiting, skipped) = pending(carrier, Utc::now()).unwrap();
        assert_eq!((waiting.len(), skipped), (1, 0));
        assert_eq!(waiting[0].request, asking);
        // A second machine cannot use the same invite.
        assert!(post_request(carrier, &invite.id, &request(folder, "other"), Utc::now()).unwrap_err().contains("already has a request"));
        // The operator answers; the new machine gets the roster and the signers.
        post_response(carrier, &invite.id, "{\"v\":1,\"files\":[]}").unwrap();
        assert_eq!(answer(carrier, &invite.id, Utc::now()).unwrap(), Answer::Approved { bundle: "{\"v\":1,\"files\":[]}".into(), signers: vec![SIGNER.into()] });
        assert_eq!(pending(carrier, Utc::now()).unwrap().0.len(), 0, "answered requests are not waiting");
        carrier.delete(&invite.id).unwrap();
        assert_eq!(answer(carrier, &invite.id, Utc::now()).unwrap(), Answer::Gone);
    }

    #[test]
    fn the_exchange_runs_start_to_finish_on_a_directory() {
        let folder = dir("exchange");
        exchange(&Directory { dir: folder.join("carrier") }, &folder);
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn an_invite_that_expired_a_request_that_does_not_verify_and_a_denial_are_handled() {
        let folder = dir("edges");
        let carrier = Directory { dir: folder.join("carrier") };
        // Expired: not offered, not accepted, swept.
        let old = post_invite(&carrier, "orchard", &[SIGNER.into()], at(-30), 15).unwrap();
        assert!(invites(&carrier, Utc::now()).unwrap().is_empty());
        assert!(post_request(&carrier, &old.id, &request(&folder, "studio"), Utc::now()).unwrap_err().contains("expired"));
        assert_eq!(answer(&carrier, &old.id, Utc::now()).unwrap(), Answer::Gone);
        assert_eq!(sweep(&carrier, Utc::now()).unwrap(), 1);
        assert!(carrier.list().unwrap().is_empty());
        // No signers, no invite.
        assert!(post_invite(&carrier, "orchard", &[], Utc::now(), 15).unwrap_err().contains("trusts no signing key"));
        // A request altered after signing is counted and never listed.
        let invite = post_invite(&carrier, "orchard", &[SIGNER.into()], Utc::now(), 15).unwrap();
        let mut forged = request(&folder, "studio");
        forged.routes[0].host = "198.51.100.1".into();
        post_request(&carrier, &invite.id, &forged, Utc::now()).unwrap();
        assert_eq!(pending(&carrier, Utc::now()).unwrap(), (Vec::new(), 1));
        // Denial reaches the new machine with its reason.
        deny(&carrier, &invite.id, "not now").unwrap();
        assert_eq!(answer(&carrier, &invite.id, Utc::now()).unwrap(), Answer::Denied("not now".into()));
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn items_that_are_not_invites_and_unsafe_ids_are_left_alone() {
        let folder = dir("foreign");
        let carrier = Directory { dir: folder.join("carrier") };
        let stranger = carrier.create(&[("kind".to_string(), "something-else".to_string())].into_iter().collect()).unwrap();
        assert!(invites(&carrier, Utc::now()).unwrap().is_empty());
        assert!(pending(&carrier, Utc::now()).unwrap().0.is_empty());
        assert_eq!(sweep(&carrier, Utc::now()).unwrap(), 0);
        assert!(carrier.get(&stranger.id).is_ok());
        for bad in ["../x", "a/b", "", "a.b"] {
            assert!(carrier.get(bad).is_err() && carrier.delete(bad).is_err(), "{bad}");
        }
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A stand-in for 1Password's `op`, keeping items as JSON files and answering the few commands the
    /// carrier uses in the shape the real one does.
    fn fake_op(folder: &Path) -> PathBuf {
        let script = folder.join("op");
        std::fs::write(
            &script,
            r#"#!/usr/bin/env python3
import json, os, sys, uuid
store = os.environ["FAKE_OP_DIR"]
os.makedirs(store, exist_ok=True)
args = sys.argv[1:]
open(os.path.join(store, "calls"), "a").write(" ".join(args[:3]) + "\n")
def load(i):
    return json.load(open(os.path.join(store, i + ".json")))
def save(item):
    json.dump(item, open(os.path.join(store, item["id"] + ".json"), "w"))
def assign(item, pairs):
    for pair in pairs:
        if "[" not in pair or "=" not in pair: continue
        name = pair.split("[", 1)[0]
        value = pair.split("=", 1)[1]
        for f in item["fields"]:
            if f["label"] == name:
                f["value"] = value; break
        else:
            item["fields"].append({"id": name, "label": name, "type": "STRING", "value": value})
if args[:2] == ["item", "create"]:
    item = {"id": uuid.uuid4().hex[:26], "title": "t", "tags": ["dojo-mesh-invite"], "fields": []}
    assign(item, [a for a in args[2:] if "[text]=" in a]); save(item); print(json.dumps(item))
elif args[:2] == ["item", "list"]:
    print(json.dumps([{"id": f[:-5], "title": "t"} for f in sorted(os.listdir(store)) if f.endswith(".json")]))
elif args[:2] == ["item", "get"]:
    try: print(json.dumps(load(args[2])))
    except FileNotFoundError: sys.stderr.write("[ERROR] isn't an item\n"); sys.exit(1)
elif args[:2] == ["item", "edit"]:
    item = load(args[2]); assign(item, args[3:]); save(item)
elif args[:2] == ["item", "delete"]:
    os.remove(os.path.join(store, args[2] + ".json"))
else:
    sys.stderr.write("unknown\n"); sys.exit(1)
"#,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[test]
    fn the_whole_exchange_runs_through_1password_s_command_line() {
        let folder = dir("op");
        let program = fake_op(&folder);
        // The fake keeps its items in a directory named by the environment of the test process.
        unsafe { std::env::set_var("FAKE_OP_DIR", folder.join("vault")) };
        let carrier = OnePassword { vault: Some("Private".into()), program };
        exchange(&carrier, &folder);
        let calls = std::fs::read_to_string(folder.join("vault/calls")).unwrap();
        assert!(calls.contains("item create") && calls.contains("item edit") && calls.contains("item delete"), "{calls}");
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn a_missing_or_locked_op_is_a_step_for_a_person() {
        let missing = OnePassword { vault: None, program: PathBuf::from("/nonexistent/op") };
        assert!(missing.list().unwrap_err().contains("not installed"));
        let folder = dir("locked");
        let script = folder.join("op");
        std::fs::write(&script, "#!/bin/sh\necho '[ERROR] you are not signed in' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let locked = OnePassword { vault: None, program: script };
        assert!(locked.list().unwrap_err().contains("unlock"), "{:?}", locked.list());
        let _ = std::fs::remove_dir_all(folder);
    }

    #[test]
    fn the_configuration_names_the_carrier() {
        use crate::config::MeshConfig;
        assert_eq!(MeshConfig::parse("").unwrap().rendezvous, None);
        assert_eq!(MeshConfig::parse("rendezvous = \"1password\"\nrendezvous-vault = \"Private\"").unwrap().rendezvous, Some(RendezvousChoice::OnePassword { vault: Some("Private".into()) }));
        assert_eq!(MeshConfig::parse("rendezvous = \"directory\"\nrendezvous-dir = \"/srv/mesh\"").unwrap().rendezvous, Some(RendezvousChoice::Directory("/srv/mesh".into())));
        for bad in ["rendezvous = \"directory\"", "rendezvous-dir = \"/x\"", "rendezvous = \"directory\"\nrendezvous-dir = \"rel\"", "rendezvous = \"carrier-pigeon\"", "rendezvous = \"1password\"\nrendezvous-dir = \"/x\""] {
            assert!(MeshConfig::parse(bad).is_err(), "{bad}");
        }
    }
}
