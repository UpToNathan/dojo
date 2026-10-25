//! Plain-text views of a roster and its findings.

use crate::doctor::{Finding, Severity};
use crate::roster::{Admission, Exclusion, Roster, Source};
use std::fmt::Write;

pub fn status(roster: &Roster, show_excluded: bool) -> String {
    let mut out = String::new();
    let peers = roster.peers().count();
    let tag = roster.membership_tag.as_deref().map(|tag| format!(" ({tag})")).unwrap_or_default();
    let _ = match peers {
        0 => writeln!(out, "Mesh of one{tag}"),
        n => writeln!(out, "Mesh of {}{tag}", n + 1),
    };
    if let Source::Unavailable(reason) = &roster.source {
        let _ = writeln!(out, "  network unavailable: {reason}");
    }
    for member in &roster.members {
        let state = if member.device.online { "online" } else { "offline" };
        let note = match member.admitted_by {
            Admission::This => " (this node)",
            Admission::Allowed => " (allowed)",
            Admission::Tag => "",
        };
        let _ = writeln!(out, "  {}{note}  {state}", member.device.name);
    }
    if show_excluded {
        for excluded in &roster.excluded {
            let why = match excluded.reason {
                Exclusion::Denied => "denied",
                Exclusion::NotAdmitted => "not admitted",
            };
            let _ = writeln!(out, "  {}  {why}", excluded.device.name);
        }
    } else if !roster.excluded.is_empty() {
        let _ = writeln!(out, "  {} other device{} not in the mesh (--all lists them)", roster.excluded.len(), if roster.excluded.len() == 1 { "" } else { "s" });
    }
    out
}

pub fn doctor(findings: &[Finding]) -> String {
    if findings.is_empty() {
        return "Healthy.\n".into();
    }
    let mut out = String::new();
    for finding in findings {
        let label = match finding.severity {
            Severity::Problem => "problem",
            Severity::Warning => "warning",
        };
        match &finding.device {
            Some(device) => writeln!(out, "{label}: {device}: {}", finding.message),
            None => writeln!(out, "{label}: {}", finding.message),
        }
        .unwrap();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MeshConfig;
    use crate::device::device;
    use crate::roster::admit;

    #[test]
    fn a_mesh_of_one_says_so() {
        let roster = admit(device("me", &[], true), vec![], &MeshConfig::default());
        assert_eq!(status(&roster, false), "Mesh of one\n  me (this node)  online\n");
    }

    #[test]
    fn lists_members_and_counts_the_rest() {
        let config = MeshConfig::parse("membership-tag = \"tag:mesh\"").unwrap();
        let roster = admit(
            device("me", &["tag:mesh"], true),
            vec![device("peer", &["tag:mesh"], false), device("stranger", &[], true)],
            &config,
        );
        assert_eq!(
            status(&roster, false),
            "Mesh of 2 (tag:mesh)\n  me (this node)  online\n  peer  offline\n  1 other device not in the mesh (--all lists them)\n"
        );
        assert!(status(&roster, true).ends_with("  stranger  not admitted\n"));
    }

    #[test]
    fn doctor_text_names_the_device() {
        let finding = Finding { severity: Severity::Problem, device: Some("peer".into()), message: "Broken.".into() };
        assert_eq!(doctor(&[finding]), "problem: peer: Broken.\n");
        assert_eq!(doctor(&[]), "Healthy.\n");
    }
}
