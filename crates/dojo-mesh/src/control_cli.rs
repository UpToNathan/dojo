//! The operator-only commands: enrolling a node and replacing the network policy.
//!
//! These change who can reach what, so they need a person: not the agent (its calls are marked),
//! and not any process without a terminal to ask at. Reading and checking a policy changes
//! nothing and is open to the agent, as the tailnet policy is the mesh's permission system.

use crate::cli::{Env, FAILED, Failure, NEEDS_HUMAN, USAGE_ERROR, fail};
use crate::config::{MeshConfig, is_tag};
use crate::control::{Client, Curl, Http, Secrets, SystemSecrets, Validation};
use crate::transport::Origin;
use crate::{diff, hujson};
use std::io::{BufRead, IsTerminal, Write};

/// Where a person is asked. The real one is the terminal; tests answer for themselves.
pub trait Terminal {
    fn interactive(&self) -> bool;
    fn confirm(&mut self, question: &str) -> bool;
}

pub struct Tty;

impl Terminal for Tty {
    fn interactive(&self) -> bool {
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
    }

    fn confirm(&mut self, question: &str) -> bool {
        eprint!("{question} [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        std::io::stdin().lock().read_line(&mut answer).is_ok() && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
    }
}

const POLICY_USAGE: &str = "usage: dojo-mesh policy show | validate FILE | apply FILE";

pub fn run(env: &Env, command: &str, rest: &[String]) -> Result<u8, Failure> {
    let config = match &env.config_path {
        Some(path) => MeshConfig::load(path).map_err(|error| fail(USAGE_ERROR, error.to_string()))?,
        None => MeshConfig::default(),
    };
    let Some(plane) = &config.control_plane else {
        return Err(fail(
            USAGE_ERROR,
            "there is no [control-plane] section in the mesh configuration; it names where the Tailscale credential is kept (see docs/mesh.md)",
        ));
    };
    let (http, secrets) = (Curl, SystemSecrets);
    let client = Client::new(&http, &secrets, plane);
    run_with(&client, &mut Tty, env.origin, &config, command, rest)
}

fn run_with(client: &Client, terminal: &mut dyn Terminal, origin: Origin, config: &MeshConfig, command: &str, rest: &[String]) -> Result<u8, Failure> {
    match (command, rest) {
        ("policy", [sub]) if sub == "show" => show(client),
        ("policy", [sub, file]) if sub == "validate" => validate(client, &read_file(file)?),
        ("policy", [sub, file]) if sub == "apply" => apply(client, terminal, origin, &read_file(file)?),
        ("policy", _) => Err(fail(USAGE_ERROR, POLICY_USAGE)),
        ("enroll", _) => enroll(client, terminal, origin, config, rest),
        _ => Err(fail(USAGE_ERROR, format!("unknown command {command}"))),
    }
}

fn read_file(path: &str) -> Result<String, Failure> {
    std::fs::read_to_string(path).map_err(|error| fail(USAGE_ERROR, format!("{path}: {error}")))
}

/// Refuses unless a person is at a terminal and the call is not the agent's.
fn require_operator(terminal: &dyn Terminal, origin: Origin, what: &str) -> Result<(), Failure> {
    if origin == Origin::Agent {
        return Err(fail(NEEDS_HUMAN, format!("{what} is for the operator; ask the operator to run it")));
    }
    if !terminal.interactive() {
        return Err(fail(NEEDS_HUMAN, format!("{what} needs a person at a terminal to confirm it")));
    }
    Ok(())
}

fn show(client: &Client) -> Result<u8, Failure> {
    let policy = client.policy().map_err(|message| fail(FAILED, message))?;
    print!("{}", policy.text);
    if !policy.text.ends_with('\n') {
        println!();
    }
    Ok(0)
}

fn validate(client: &Client, text: &str) -> Result<u8, Failure> {
    let json = hujson::standardize(text).map_err(|message| fail(USAGE_ERROR, message))?;
    match client.validate(&json).map_err(|message| fail(FAILED, message))? {
        Validation::Valid => {
            println!("Valid: the policy parses and its tests pass.");
            Ok(0)
        }
        Validation::Invalid(why) => Err(fail(FAILED, format!("the policy was rejected:\n{why}"))),
    }
}

fn apply(client: &Client, terminal: &mut dyn Terminal, origin: Origin, text: &str) -> Result<u8, Failure> {
    require_operator(terminal, origin, "replacing the network policy")?;
    let json = hujson::standardize(text).map_err(|message| fail(USAGE_ERROR, message))?;
    if let Validation::Invalid(why) = client.validate(&json).map_err(|message| fail(FAILED, message))? {
        return Err(fail(FAILED, format!("the policy was rejected, so nothing was changed:\n{why}")));
    }
    let current = client.policy().map_err(|message| fail(FAILED, message))?;
    let Some(etag) = current.etag.as_deref() else {
        return Err(fail(FAILED, "Tailscale gave no ETag for the current policy, so a safe replacement is not possible"));
    };
    let Some(changes) = diff::render(&current.text, text, 3) else {
        println!("The network policy already matches this file.");
        return Ok(0);
    };
    println!("{changes}");
    println!("The new policy is valid and its tests pass.");
    if !terminal.confirm("Replace the network policy with this?") {
        println!("Not changed.");
        return Ok(1);
    }
    client.set_policy(text, etag).map_err(|message| fail(FAILED, message))?;
    println!("The network policy was replaced.");
    Ok(0)
}

const DEFAULT_EXPIRY: u64 = 3600;
const LONGEST_EXPIRY: u64 = 90 * 24 * 3600;

fn enroll(client: &Client, terminal: &mut dyn Terminal, origin: Origin, config: &MeshConfig, rest: &[String]) -> Result<u8, Failure> {
    require_operator(terminal, origin, "enrolling a node")?;
    let mut tags: Vec<String> = Vec::new();
    let mut expiry = DEFAULT_EXPIRY;
    let mut args = rest.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--tag" => tags.push(args.next().ok_or_else(|| fail(USAGE_ERROR, "--tag needs a value"))?.clone()),
            "--expires" => {
                let text = args.next().ok_or_else(|| fail(USAGE_ERROR, "--expires needs a number of seconds"))?;
                expiry = text.parse().map_err(|_| fail(USAGE_ERROR, format!("--expires needs whole seconds, not {text:?}")))?;
            }
            other => return Err(fail(USAGE_ERROR, format!("unknown option {other}"))),
        }
    }
    if tags.is_empty() {
        tags.extend(config.membership_tag.clone());
    }
    if tags.is_empty() {
        return Err(fail(USAGE_ERROR, "enroll needs a tag: pass --tag or set membership-tag in the mesh configuration"));
    }
    if let Some(bad) = tags.iter().find(|tag| !is_tag(tag)) {
        return Err(fail(USAGE_ERROR, format!("{bad:?} is not a Tailscale tag; expected something like \"tag:name\"")));
    }
    if !(60..=LONGEST_EXPIRY).contains(&expiry) {
        return Err(fail(USAGE_ERROR, format!("--expires must be between 60 seconds and {LONGEST_EXPIRY} (90 days)")));
    }
    let question = format!("Create a one-use key that enrols a device as {} and expires in {}?", tags.join(", "), human(expiry));
    if !terminal.confirm(&question) {
        eprintln!("Not created.");
        return Ok(1);
    }
    let key = client.create_auth_key(&tags, expiry).map_err(|message| fail(FAILED, message))?;
    println!("{}", key.key);
    eprintln!("On the new machine: tailscale up --auth-key=<the key above>{}", key.expires.map(|at| format!("   (expires {at})")).unwrap_or_default());
    Ok(0)
}

fn human(seconds: u64) -> String {
    match seconds {
        s if s % 86400 == 0 => format!("{} day{}", s / 86400, if s == 86400 { "" } else { "s" }),
        s if s % 3600 == 0 => format!("{} hour{}", s / 3600, if s == 3600 { "" } else { "s" }),
        s if s % 60 == 0 => format!("{} minutes", s / 60),
        s => format!("{s} seconds"),
    }
}

// The client takes its collaborators as trait objects; this keeps the unused-import lint honest
// for builds without tests.
#[allow(dead_code)]
fn _assert_object_safe(_: &dyn Http, _: &dyn Secrets) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ControlPlane, Credential, SecretRef};
    use crate::control::testing::{FakeHttp, FakeSecrets};
    use crate::control::{Request, Response};
    use std::collections::HashMap;

    struct Person {
        at_terminal: bool,
        says_yes: bool,
        asked: Vec<String>,
    }

    impl Person {
        fn yes() -> Self {
            Self { at_terminal: true, says_yes: true, asked: Vec::new() }
        }
    }

    impl Terminal for Person {
        fn interactive(&self) -> bool {
            self.at_terminal
        }

        fn confirm(&mut self, question: &str) -> bool {
            self.asked.push(question.to_string());
            self.says_yes
        }
    }

    fn reply(status: u16, body: &str) -> Response {
        Response { status, headers: vec![("ETag".into(), "\"e1\"".into())], body: body.into() }
    }

    fn plane() -> (ControlPlane, FakeSecrets) {
        let reference = SecretRef::Env("T".into());
        (ControlPlane { tailnet: "-".into(), credential: Credential::ApiToken(reference.clone()) }, FakeSecrets(HashMap::from([(reference, "tok".to_string())])))
    }

    fn sent(http: &FakeHttp) -> Vec<Request> {
        http.sent.borrow().clone()
    }

    const CURRENT: &str = "{\n  // keep\n  \"acls\": [],\n}\n";
    const NEXT: &str = "{\n  // keep\n  \"acls\": [{\"action\": \"accept\"}],\n}\n";

    #[test]
    fn apply_validates_shows_the_change_asks_and_then_replaces_with_the_etag() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(200, CURRENT), reply(200, "{}")]);
        let mut person = Person::yes();
        let client = Client::new(&http, &secrets, &plane);
        assert_eq!(apply(&client, &mut person, Origin::Operator, NEXT).unwrap(), 0);
        let calls = sent(&http);
        assert_eq!(calls.len(), 3);
        assert!(calls[0].url.ends_with("/acl/validate"), "validated first");
        assert!(!calls[0].body.as_deref().unwrap().contains("keep"), "the check gets plain JSON");
        assert!(calls[2].url.ends_with("/acl") && calls[2].headers.contains(&("If-Match".to_string(), "\"e1\"".to_string())));
        assert_eq!(calls[2].body.as_deref(), Some(NEXT), "the comments are kept when the policy is replaced");
        assert_eq!(person.asked.len(), 1);
    }

    #[test]
    fn declining_changes_nothing() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(200, CURRENT)]);
        let mut person = Person { says_yes: false, ..Person::yes() };
        assert_eq!(apply(&Client::new(&http, &secrets, &plane), &mut person, Origin::Operator, NEXT).unwrap(), 1);
        assert_eq!(sent(&http).len(), 2, "no write was sent");
    }

    #[test]
    fn a_rejected_policy_is_never_offered_for_approval() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(400, "{\"message\": \"unknown field\"}")]);
        let mut person = Person::yes();
        let failure = apply(&Client::new(&http, &secrets, &plane), &mut person, Origin::Operator, NEXT).unwrap_err();
        assert_eq!(failure.code, FAILED);
        assert!(failure.message.contains("unknown field") && failure.message.contains("nothing was changed"), "{}", failure.message);
        assert!(person.asked.is_empty());
        assert_eq!(sent(&http).len(), 1);
    }

    #[test]
    fn an_identical_policy_is_left_alone_without_asking() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(200, CURRENT)]);
        let mut person = Person::yes();
        assert_eq!(apply(&Client::new(&http, &secrets, &plane), &mut person, Origin::Operator, CURRENT).unwrap(), 0);
        assert!(person.asked.is_empty());
    }

    #[test]
    fn changes_need_a_person_who_is_the_operator() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![]);
        let client = Client::new(&http, &secrets, &plane);
        let agent = apply(&client, &mut Person::yes(), Origin::Agent, NEXT).unwrap_err();
        assert_eq!(agent.code, NEEDS_HUMAN);
        let headless = apply(&client, &mut Person { at_terminal: false, ..Person::yes() }, Origin::Operator, NEXT).unwrap_err();
        assert_eq!(headless.code, NEEDS_HUMAN);
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"").unwrap();
        assert_eq!(enroll(&client, &mut Person::yes(), Origin::Agent, &config, &[]).unwrap_err().code, NEEDS_HUMAN);
        assert_eq!(enroll(&client, &mut Person { at_terminal: false, ..Person::yes() }, Origin::Operator, &config, &[]).unwrap_err().code, NEEDS_HUMAN);
        assert!(sent(&http).is_empty(), "nothing reached the network");
    }

    #[test]
    fn enroll_uses_the_membership_tag_and_asks_first() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(200, "{\"key\": \"tskey-auth-xyz\"}")]);
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"").unwrap();
        let mut person = Person::yes();
        assert_eq!(enroll(&Client::new(&http, &secrets, &plane), &mut person, Origin::Operator, &config, &["--expires".into(), "7200".into()]).unwrap(), 0);
        assert_eq!(person.asked, ["Create a one-use key that enrols a device as tag:mesh and expires in 2 hours?"]);
        let body: serde_json::Value = serde_json::from_str(sent(&http)[0].body.as_deref().unwrap()).unwrap();
        assert_eq!(body["capabilities"]["devices"]["create"]["tags"], serde_json::json!(["tag:mesh"]));
        assert_eq!(body["expirySeconds"], 7200);
    }

    #[test]
    fn enroll_refuses_bad_input_before_asking_or_calling() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![]);
        let client = Client::new(&http, &secrets, &plane);
        let none = MeshConfig::default();
        for args in [vec![], vec!["--tag", "mesh"], vec!["--tag", "tag:x", "--expires", "5"], vec!["--tag", "tag:x", "--expires", "999999999"], vec!["--tag", "tag:x", "--expires", "soon"], vec!["--bogus"]] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let mut person = Person::yes();
            let failure = enroll(&client, &mut person, Origin::Operator, &none, &args).unwrap_err();
            assert_eq!(failure.code, USAGE_ERROR, "{args:?}: {}", failure.message);
            assert!(person.asked.is_empty(), "{args:?}");
        }
        assert!(sent(&http).is_empty());
    }

    #[test]
    fn declining_enrolment_creates_no_key() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![]);
        let mut person = Person { says_yes: false, ..Person::yes() };
        let args = vec!["--tag".to_string(), "tag:x".to_string()];
        assert_eq!(enroll(&Client::new(&http, &secrets, &plane), &mut person, Origin::Operator, &MeshConfig::default(), &args).unwrap(), 1);
        assert!(sent(&http).is_empty());
    }

    #[test]
    fn validate_and_show_are_open_to_the_agent_and_need_no_terminal() {
        let (plane, secrets) = plane();
        let http = FakeHttp::new(vec![reply(200, "{}"), reply(200, CURRENT)]);
        let client = Client::new(&http, &secrets, &plane);
        let config = MeshConfig::default();
        let mut headless = Person { at_terminal: false, ..Person::yes() };
        let validate_args = vec!["validate".to_string(), "/dev/null".to_string()];
        assert!(run_with(&client, &mut headless, Origin::Agent, &config, "policy", &validate_args).is_err(), "an empty file is not HuJSON");
        assert_eq!(validate(&client, NEXT).unwrap(), 0);
        assert_eq!(run_with(&client, &mut headless, Origin::Agent, &config, "policy", &["show".to_string()]).unwrap(), 0);
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(human(3600), "1 hour");
        assert_eq!(human(7200), "2 hours");
        assert_eq!(human(86400), "1 day");
        assert_eq!(human(90), "90 seconds");
        assert_eq!(human(300), "5 minutes");
    }
}
