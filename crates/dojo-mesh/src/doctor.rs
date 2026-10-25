use crate::config::MeshConfig;
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
    let tailscale = matches!(roster.source, Source::Tailscale);
    for rejected in &roster.rejected {
        findings.push(finding(Severity::Warning, None, format!("Ignored {}: {}", rejected.label, rejected.reason)));
    }
    match &config.membership_tag {
        _ if !tailscale => {}
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
        let what = if tailscale { "Its Tailscale key" } else { "Its roster entry" };
        if device.key_expired {
            findings.push(finding(Severity::Problem, name, format!("{what} has expired.")));
        } else if let Some(expiry) = device.key_expiry {
            let left = expiry - now;
            if left <= EXPIRY_WARNING {
                let days = left.num_days().max(0);
                findings.push(finding(Severity::Warning, name, format!("{what} expires in {days} day{}.", if days == 1 { "" } else { "s" })));
            }
        }
    }
    for (list, entries) in [("allow", &config.allow), ("deny", &config.deny)] {
        for entry in entries.iter().filter(|_| tailscale || list == "deny") {
            let seen = roster.members.iter().map(|m| &m.device).chain(roster.excluded.iter().map(|e| &e.device)).any(|d| d.answers_to(entry));
            if !seen {
                findings.push(finding(Severity::Warning, Some(entry), format!("Listed in {list}, but no device by that name is on the network.")));
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
}
