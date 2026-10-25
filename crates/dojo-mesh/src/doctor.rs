use crate::capability::Description;
use crate::config::{Boundary, MeshConfig, name_key};
use crate::device::Device;
use crate::roster::{Admission, Roster, Source};
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// Keys that lapse within this long are worth a warning.
const EXPIRY_WARNING: Duration = Duration::days(14);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    /// Worth knowing; the mesh works.
    Warning,
    /// Something the mesh needs is broken.
    Problem,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub severity: Severity,
    /// The device the finding is about, when it is about one.
    pub device: Option<String>,
    pub message: String,
}

fn finding(severity: Severity, device: Option<&str>, message: impl Into<String>) -> Finding {
    Finding { severity, device: device.map(str::to_string), message: message.into() }
}

/// Everything wrong or doubtful about a roster. A healthy mesh, including a mesh of one, returns nothing.
pub fn diagnose(roster: &Roster, config: &MeshConfig, now: DateTime<Utc>) -> Vec<Finding> {
    let mut findings = Vec::new();
    if let Source::Unavailable(reason) = &roster.source {
        findings.push(finding(Severity::Warning, None, format!("The network cannot be read ({reason}); this node is a mesh of one.")));
        return findings;
    }
    let this = roster.this();
    match &config.membership_tag {
        None if config.allow.is_empty() => findings.push(finding(
            Severity::Warning,
            None,
            "No membership tag or allow list is configured, so this node admits no peers.",
        )),
        Some(tag) if !this.has_tag(tag) => findings.push(finding(
            Severity::Problem,
            Some(&this.name),
            format!("This node does not carry {tag}, so nodes that use it will not admit this one."),
        )),
        _ => {}
    }
    for member in &roster.members {
        let device = &member.device;
        let name = Some(device.name.as_str());
        if member.admitted_by == Admission::Allowed && config.membership_tag.as_deref().is_some_and(|tag| !device.has_tag(tag)) {
            findings.push(finding(Severity::Warning, name, "Admitted by the allow list only; it does not carry the membership tag."));
        }
        if !device.online && member.admitted_by != Admission::This {
            findings.push(finding(Severity::Warning, name, "Offline."));
        }
        if device.key_expired {
            findings.push(finding(Severity::Problem, name, "Its Tailscale key has expired."));
        } else if let Some(expiry) = device.key_expiry {
            let left = expiry - now;
            if left <= EXPIRY_WARNING {
                let days = left.num_days().max(0);
                findings.push(finding(Severity::Warning, name, format!("Its Tailscale key expires in {days} day{}.", if days == 1 { "" } else { "s" })));
            }
        }
    }
    for (list, entries) in [("allow", &config.allow), ("deny", &config.deny)] {
        for entry in entries {
            let seen = roster.members.iter().map(|m| &m.device).chain(roster.excluded.iter().map(|e| &e.device)).any(|d| d.answers_to(entry));
            if !seen {
                findings.push(finding(Severity::Warning, Some(entry), format!("Listed in {list}, but no device by that name is on the network.")));
            }
        }
    }
    findings
}

/// What is known about the peer at the other end of a boundary.
pub enum PeerState {
    NotMember,
    Offline,
    /// It is a member and online but could not be asked.
    Unreadable(String),
    Described(Description),
}

pub struct PeerView {
    /// The peer as the boundary names it.
    pub name: String,
    pub state: PeerState,
}

fn first_label(name: &str) -> &str {
    name.split('.').next().unwrap_or(name)
}

fn same_node(a: &str, b: &str) -> bool {
    let (a, b) = (name_key(a), name_key(b));
    a == b || first_label(&a) == first_label(&b)
}

fn matches_back(theirs: &Boundary, ours: &Boundary, this: &Device) -> bool {
    this.answers_to(&theirs.peer) && theirs.output == ours.peer_output && theirs.edge == ours.peer_edge && theirs.peer_output == ours.output && theirs.peer_edge == ours.edge
}

/// What is wrong with the configured boundaries: outputs that do not exist, a peer that does not
/// declare the boundary back, or input that one side does not accept. A boundary needs both ends,
/// so the peer's own description is part of the answer. `outputs` is this node's, when known.
pub fn diagnose_boundaries(this: &Device, config: &MeshConfig, outputs: Option<&[String]>, peers: &[PeerView]) -> Vec<Finding> {
    let mut findings: Vec<Finding> = Vec::new();
    let mut add = |finding: Finding| {
        if !findings.contains(&finding) {
            findings.push(finding);
        }
    };
    for boundary in &config.boundaries {
        let label = format!("Boundary {} {} to {}", boundary.output, boundary.edge.as_str(), boundary.peer);
        if outputs.is_some_and(|outputs| !outputs.contains(&boundary.output)) {
            add(finding(Severity::Problem, Some(&this.name), format!("{label}: this node has no output {}.", boundary.output)));
        }
        if !config.accept_input.iter().any(|accepted| same_node(accepted, &boundary.peer)) {
            add(finding(Severity::Problem, Some(&this.name), format!("{label}: this node does not accept input from {}; add it to accept-input.", boundary.peer)));
        }
        let Some(view) = peers.iter().find(|view| same_node(&view.name, &boundary.peer)) else { continue };
        match &view.state {
            PeerState::NotMember => add(finding(Severity::Problem, Some(&boundary.peer), format!("{label}: {} is not a member of this mesh, so the boundary cannot work.", boundary.peer))),
            PeerState::Offline => add(finding(Severity::Warning, Some(&boundary.peer), format!("{label}: offline, so the other end was not checked."))),
            PeerState::Unreadable(why) => add(finding(Severity::Warning, Some(&boundary.peer), format!("{label}: the other end could not be checked ({why})."))),
            PeerState::Described(theirs) => {
                if theirs.outputs.as_ref().is_some_and(|outputs| !outputs.contains(&boundary.peer_output)) {
                    add(finding(Severity::Problem, Some(&boundary.peer), format!("{label}: {} has no output {}.", boundary.peer, boundary.peer_output)));
                }
                if !theirs.boundaries.iter().any(|back| matches_back(back, boundary, this)) {
                    add(finding(
                        Severity::Problem,
                        Some(&boundary.peer),
                        format!("{label}: {} does not declare the matching boundary back (output {}, edge {}, peer {}, peer-output {}, peer-edge {}).", boundary.peer, boundary.peer_output, boundary.peer_edge.as_str(), this.name, boundary.output, boundary.edge.as_str()),
                    ));
                }
                if !theirs.accept_input.iter().any(|accepted| this.answers_to(accepted)) {
                    add(finding(Severity::Problem, Some(&boundary.peer), format!("{label}: {} does not accept input from this node.", boundary.peer)));
                }
            }
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::device;
    use crate::roster::admit;

    fn now() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-01-10T00:00:00Z").unwrap().with_timezone(&Utc)
    }

    fn config(text: &str) -> MeshConfig {
        MeshConfig::parse(text).unwrap()
    }

    fn messages(findings: &[Finding]) -> Vec<String> {
        findings.iter().map(|f| format!("{:?} {} {}", f.severity, f.device.as_deref().unwrap_or("-"), f.message)).collect()
    }

    #[test]
    fn a_healthy_mesh_has_no_findings() {
        let config = config("membership-tag = \"tag:mesh\"");
        let roster = admit(device("me", &["tag:mesh"], true), vec![device("peer", &["tag:mesh"], true)], &config);
        assert_eq!(diagnose(&roster, &config, now()), []);
    }

    #[test]
    fn an_unreadable_network_is_a_warning_not_a_failure() {
        let config = MeshConfig::default();
        let roster = Roster::alone(device("me", &[], true), &config, "tailscale is not installed".into());
        let findings = diagnose(&roster, &config, now());
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
    }

    #[test]
    fn an_unconfigured_mesh_says_why_it_is_alone() {
        let config = MeshConfig::default();
        let roster = admit(device("me", &[], true), vec![], &config);
        let found = messages(&diagnose(&roster, &config, now()));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("admits no peers"));
    }

    #[test]
    fn this_node_missing_the_tag_is_a_problem() {
        let config = config("membership-tag = \"tag:mesh\"");
        let roster = admit(device("me", &[], true), vec![], &config);
        let findings = diagnose(&roster, &config, now());
        assert_eq!(findings[0].severity, Severity::Problem);
        assert_eq!(findings[0].device.as_deref(), Some("me"));
    }

    #[test]
    fn offline_and_expiring_and_expired_peers_are_reported() {
        let config = config("membership-tag = \"tag:mesh\"");
        let mut soon = device("soon", &["tag:mesh"], true);
        soon.key_expiry = Some(now() + Duration::days(3) + Duration::hours(1));
        let mut gone = device("gone", &["tag:mesh"], false);
        gone.key_expired = true;
        let mut fine = device("fine", &["tag:mesh"], true);
        fine.key_expiry = Some(now() + Duration::days(200));
        let roster = admit(device("me", &["tag:mesh"], true), vec![soon, gone, fine], &config);
        let found = messages(&diagnose(&roster, &config, now()));
        assert_eq!(
            found,
            [
                "Warning gone Offline.",
                "Problem gone Its Tailscale key has expired.",
                "Warning soon Its Tailscale key expires in 3 days.",
            ]
        );
    }

    #[test]
    fn stale_overrides_and_untagged_allowed_peers_are_flagged() {
        let config = config("membership-tag = \"tag:mesh\"\nallow = [\"odd\", \"ghost\"]\ndeny = [\"phantom\"]");
        let roster = admit(device("me", &["tag:mesh"], true), vec![device("odd", &[], true)], &config);
        let found = messages(&diagnose(&roster, &config, now()));
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.iter().any(|m| m.contains("odd") && m.contains("allow list only")));
        assert!(found.iter().any(|m| m.contains("ghost") && m.contains("Listed in allow")));
        assert!(found.iter().any(|m| m.contains("phantom") && m.contains("Listed in deny")));
    }

    mod boundaries {
        use super::*;
        use crate::config::Edge;

        fn this() -> Device {
            device("alpha", &[], true)
        }

        fn config(accept: &str) -> MeshConfig {
            MeshConfig::parse(&format!(
                "accept-input = [{accept}]\n[[boundary]]\noutput = \"eDP-1\"\nedge = \"right\"\npeer = \"beta\"\npeer-output = \"DP-1\"\npeer-edge = \"left\"\n"
            ))
            .unwrap()
        }

        fn back() -> Boundary {
            Boundary { output: "DP-1".into(), edge: Edge::Left, peer: "alpha".into(), peer_output: "eDP-1".into(), peer_edge: Edge::Right }
        }

        fn described(boundaries: Vec<Boundary>, accept: &[&str], outputs: Option<&[&str]>) -> PeerView {
            PeerView {
                name: "beta".into(),
                state: PeerState::Described(Description {
                    node: "beta".into(),
                    boundaries,
                    accept_input: accept.iter().map(|a| a.to_string()).collect(),
                    outputs: outputs.map(|o| o.iter().map(|n| n.to_string()).collect()),
                    ..Description::default()
                }),
            }
        }

        fn found(findings: &[Finding]) -> Vec<String> {
            findings.iter().map(|f| format!("{:?} {}", f.severity, f.message)).collect()
        }

        #[test]
        fn matching_boundaries_accepted_both_ways_are_clean() {
            let peer = described(vec![back()], &["alpha"], Some(&["DP-1"]));
            let outputs = vec!["eDP-1".to_string()];
            assert_eq!(diagnose_boundaries(&this(), &config("\"beta\""), Some(&outputs), &[peer]), []);
        }

        #[test]
        fn a_one_sided_boundary_says_what_the_peer_must_declare() {
            let peer = described(vec![], &["alpha"], None);
            let found = found(&diagnose_boundaries(&this(), &config("\"beta\""), None, &[peer]));
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(found[0].starts_with("Problem") && found[0].contains("does not declare the matching boundary back"), "{found:?}");
            assert!(found[0].contains("output DP-1, edge left, peer alpha, peer-output eDP-1, peer-edge right"), "{found:?}");
        }

        #[test]
        fn a_back_boundary_for_different_edges_does_not_count() {
            let mut wrong = back();
            wrong.peer_edge = Edge::Top;
            let peer = described(vec![wrong], &["alpha"], None);
            assert!(found(&diagnose_boundaries(&this(), &config("\"beta\""), None, &[peer])).iter().any(|m| m.contains("matching boundary back")));
        }

        #[test]
        fn input_must_be_accepted_at_both_ends() {
            let peer = described(vec![back()], &[], None);
            let found = found(&diagnose_boundaries(&this(), &config(""), None, &[peer]));
            assert_eq!(found.len(), 2, "{found:?}");
            assert!(found.iter().any(|m| m.contains("this node does not accept input from beta")));
            assert!(found.iter().any(|m| m.contains("beta does not accept input from this node")));
        }

        #[test]
        fn accept_input_matches_a_full_network_name() {
            let peer = described(vec![back()], &["alpha.example.ts.net"], None);
            assert_eq!(diagnose_boundaries(&this(), &config("\"beta.example.ts.net\""), None, &[peer]), []);
        }

        #[test]
        fn unknown_outputs_are_problems_when_the_outputs_are_known() {
            let peer = described(vec![back()], &["alpha"], Some(&["HDMI-A-1"]));
            let outputs = vec!["DP-9".to_string()];
            let found = found(&diagnose_boundaries(&this(), &config("\"beta\""), Some(&outputs), &[peer]));
            assert!(found.iter().any(|m| m.contains("this node has no output eDP-1")), "{found:?}");
            assert!(found.iter().any(|m| m.contains("beta has no output DP-1")), "{found:?}");
        }

        #[test]
        fn a_peer_that_cannot_be_checked_is_a_warning_and_a_stranger_is_a_problem() {
            let cfg = config("\"beta\"");
            for (state, severity) in [(PeerState::Offline, "Warning"), (PeerState::Unreadable("timed out".into()), "Warning"), (PeerState::NotMember, "Problem")] {
                let found = found(&diagnose_boundaries(&this(), &cfg, None, &[PeerView { name: "beta".into(), state }]));
                assert_eq!(found.len(), 1, "{found:?}");
                assert!(found[0].starts_with(severity), "{found:?}");
            }
        }

        #[test]
        fn no_boundaries_means_nothing_to_report() {
            assert_eq!(diagnose_boundaries(&this(), &MeshConfig::default(), None, &[]), []);
        }
    }
}
