//! What the shell's mesh panel draws, read in one go and computed nowhere else: a card per machine,
//! the health findings `doctor` would print, whether the key holder is ready, the signing keys this
//! node trusts and the join requests waiting for it. The panel changes nothing itself; every change is
//! the `dojo-mesh` command that does it, so the terminal and the panel cannot disagree.

use crate::cli::{self, Env};
use crate::config::{MeshConfig, MethodChoice, RosterPaths, roster_paths};
use crate::doctor::{Finding, Severity, diagnose};
use crate::enroll;
use crate::roster::Roster;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;

/// An entry this close to expiring is shown as needing attention.
const EXPIRING_DAYS: i64 = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
    Tailscale,
    Roster,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Card {
    pub name: String,
    pub this: bool,
    pub online: bool,
    /// Whole days until the roster entry expires, rounded down; negative once it has. `None` for a method whose
    /// members do not expire this way.
    pub days_left: Option<i64>,
    pub expiring: bool,
    /// Where it is reached, as the first route shows it.
    pub route: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Signer {
    pub fingerprint: String,
    pub hardware: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Panel {
    pub kind: Kind,
    pub this: String,
    pub cards: Vec<Card>,
    pub findings: Vec<Finding>,
    /// Why peers cannot be reached from here yet (1Password locked or off), when that is so.
    pub blocked: Option<String>,
    /// Whether this node has a signing key to sign with.
    pub can_sign: bool,
    pub signers: Vec<Signer>,
}

/// A card per member, this machine first.
pub fn cards(roster: &Roster, now: DateTime<Utc>) -> Vec<Card> {
    roster
        .members
        .iter()
        .enumerate()
        .map(|(index, member)| {
            let device = &member.device;
            // Floored, so an entry that expired an hour ago reads -1 and not "0 days left".
            let days_left = device.key_expiry.map(|expiry| (expiry - now).num_seconds().div_euclid(86_400));
            Card {
                name: device.name.clone(),
                this: index == 0,
                online: device.online,
                days_left,
                expiring: days_left.is_some_and(|days| days <= EXPIRING_DAYS),
                route: device.routes.first().map(|route| match route {
                    crate::method::Route::Host { name, port: Some(port) } => format!("{name}:{port}"),
                    crate::method::Route::Host { name, port: None } => name.clone(),
                    crate::method::Route::Command { .. } => "via a command".to_string(),
                }),
            }
        })
        .collect()
}

fn signers(paths: &RosterPaths) -> Vec<Signer> {
    enroll::signers(&std::fs::read_to_string(&paths.signers).unwrap_or_default())
        .into_iter()
        .filter_map(|key| Some(Signer { fingerprint: enroll::fingerprint(&key).ok()?, hardware: key.starts_with("sk-") }))
        .collect()
}

/// The panel's state for this node now.
pub fn read() -> Panel {
    match Env::from_process(None) {
        Ok(env) => {
            let config = cli::load_config(&env).unwrap_or_default();
            read_with(&env, &config)
        }
        Err(_) => Panel { kind: Kind::Tailscale, this: String::new(), cards: Vec::new(), findings: Vec::new(), blocked: None, can_sign: false, signers: Vec::new() },
    }
}

pub fn read_with(env: &Env, config: &MeshConfig) -> Panel {
    let now = Utc::now();
    let roster = cli::load_roster(env, config);
    let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
    let paths = roster_paths(config, var("XDG_DATA_HOME"), var("XDG_CONFIG_HOME"), env.home.clone());
    let roster_method = config.method == MethodChoice::Roster;
    let mut findings = diagnose(&roster, config, now);
    if roster_method {
        findings.extend(crate::enroll_cli::custody_findings(env, config));
    }
    let blocked = env.method.channel.blocked();
    if let Some(step) = &blocked {
        findings.push(Finding { severity: Severity::Warning, device: None, message: format!("Peers cannot be reached from here yet: {step}.") });
    }
    let signers = match (&paths, roster_method) {
        (Some(paths), true) => signers(paths),
        _ => Vec::new(),
    };
    Panel {
        kind: if roster_method { Kind::Roster } else { Kind::Tailscale },
        this: roster.this().name.clone(),
        cards: cards(&roster, now),
        findings,
        blocked,
        can_sign: roster_method && !config.signing_keys.is_empty(),
        signers,
    }
}

/// A machine that asked to join through an invite and waits for an answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Asking {
    pub invite: String,
    pub name: String,
}

/// The state of the invites this node made: whether one is open (and until when) and who is asking.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Exchange {
    /// The open invites made by this node: id and when each expires.
    pub open: Vec<(String, DateTime<Utc>)>,
    pub asking: Vec<Asking>,
    /// Why the carrier could not be read (1Password locked, say).
    pub error: Option<String>,
}

/// What the invite exchange looks like now. It asks the carrier, which for 1Password runs `op`, so it is
/// read only while the panel is showing the Add machine view.
pub fn exchange() -> Exchange {
    let Ok(env) = Env::from_process(None) else { return Exchange::default() };
    let config = cli::load_config(&env).unwrap_or_default();
    exchange_with(&crate::enroll_cli::node_name(&env, &config), &*crate::rendezvous::open(&config.rendezvous), Utc::now())
}

pub fn exchange_with(this: &str, carrier: &dyn crate::rendezvous::Rendezvous, now: DateTime<Utc>) -> Exchange {
    let mut found = Exchange::default();
    match crate::rendezvous::invites(carrier, now) {
        Ok(invites) => {
            found.open = invites.iter().filter(|i| i.operator == this && i.status == crate::rendezvous::Status::Open).map(|i| (i.id.clone(), i.expires)).collect();
        }
        Err(error) => {
            found.error = Some(error);
            return found;
        }
    }
    match crate::rendezvous::pending(carrier, now) {
        Ok((pending, _)) => found.asking = pending.into_iter().filter(|p| p.invite.operator == this).map(|p| Asking { invite: p.invite.id, name: p.request.name }).collect(),
        Err(error) => found.error = Some(error),
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use crate::device::Device;
    use crate::roster::{Admission, Member};

    fn device(name: &str, online: bool, expires_in_days: Option<i64>) -> Device {
        Device {
            name: name.into(),
            dns_name: None,
            tags: Vec::new(),
            online,
            key_expiry: expires_in_days.map(|days| Utc::now() + chrono::Duration::days(days) + chrono::Duration::hours(1)),
            key_expired: false,
            routes: vec![crate::method::Route::Host { name: "192.0.2.7".into(), port: Some(2222) }],
            keys: None,
        }
    }

    fn roster(devices: Vec<Device>) -> Roster {
        let mut members = devices.into_iter().enumerate().map(|(index, device)| Member { device, admitted_by: if index == 0 { Admission::This } else { Admission::Signed } });
        let this = members.next().unwrap();
        Roster { source: crate::roster::Source::Method("roster".into()), membership_tag: None, members: std::iter::once(this).chain(members).collect(), excluded: Vec::new(), rejected: Vec::new() }
    }

    #[test]
    fn a_card_per_machine_with_this_one_first_days_left_and_where_it_is_reached() {
        let cards = cards(&roster(vec![device("alpha", true, Some(80)), device("beta", false, Some(5)), device("gone", true, Some(-3))]), Utc::now());
        assert_eq!(cards.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["alpha", "beta", "gone"]);
        assert!(cards[0].this && !cards[1].this);
        assert_eq!((cards[0].days_left, cards[0].expiring), (Some(80), false));
        assert_eq!((cards[1].days_left, cards[1].expiring, cards[1].online), (Some(5), true, false));
        assert_eq!(cards[2].days_left, Some(-3), "an expired entry counts down past zero");
        assert!(cards[2].expiring);
        assert_eq!(cards[0].route.as_deref(), Some("192.0.2.7:2222"));
    }

    #[test]
    fn a_method_without_expiry_shows_none() {
        let cards = cards(&roster(vec![device("alpha", true, None), device("beta", true, None)]), Utc::now());
        assert!(cards.iter().all(|c| c.days_left.is_none() && !c.expiring));
    }

    use crate::rendezvous::{self, Directory};

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-panel-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_exchange_shows_this_nodes_open_invite_and_who_is_asking_and_nobody_elses() {
        let dir = folder("exchange");
        let carrier = Directory { dir: dir.join("carrier") };
        const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(exchange_with("orchard", &carrier, Utc::now()), Exchange::default());
        let mine = rendezvous::post_invite(&carrier, "orchard", &[KEY.into()], Utc::now(), 15).unwrap();
        let theirs = rendezvous::post_invite(&carrier, "harbor", &[KEY.into()], Utc::now(), 15).unwrap();
        let shown = exchange_with("orchard", &carrier, Utc::now());
        assert_eq!(shown.open.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(), [mine.id.clone()], "another operator's invite is not ours");
        assert!(shown.asking.is_empty() && shown.error.is_none());
        let _ = theirs;
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_unreadable_carrier_is_reported_not_hidden() {
        struct Broken;
        impl rendezvous::Rendezvous for Broken {
            fn create(&self, _: &BTreeMap<String, String>) -> Result<rendezvous::Item, String> { Err("1Password needs you first: unlock it".into()) }
            fn list(&self) -> Result<Vec<rendezvous::Item>, String> { Err("1Password needs you first: unlock it".into()) }
            fn get(&self, _: &str) -> Result<rendezvous::Item, String> { Err("x".into()) }
            fn update(&self, _: &str, _: &BTreeMap<String, String>) -> Result<(), String> { Err("x".into()) }
            fn delete(&self, _: &str) -> Result<(), String> { Err("x".into()) }
        }
        let shown = exchange_with("orchard", &Broken, Utc::now());
        assert!(shown.error.as_deref().is_some_and(|error| error.contains("unlock")));
    }
}
