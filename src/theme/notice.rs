//! The notice that ends a theme apply the person started (the picker, `dojo-theme apply`,
//! `preset`, `select`). The automatic appearance refresh sends none.

use dojo_notify::{Notice, Urgency};

use super::ApplyReport;

/// How many target names the body spells out before it counts the rest.
const NAMED: usize = 3;

/// Statuses that mean a target did not take the theme and the person may need to act. Every other
/// status (`published`, `ready`, `reload_requested`, `needs_selection`) is a good state.
fn is_problem(status: &str) -> bool {
    matches!(status, "blocked" | "reload_failed" | "needs_setup")
}

/// Low "Theme applied" when every target is in a good state; otherwise Normal "Theme applied with
/// problems", the body naming the targets that have one.
pub fn notice(report: &ApplyReport) -> Notice {
    let mut problems: Vec<&str> = Vec::new();
    for target in report.targets.iter().filter(|t| is_problem(&t.status)) {
        if !problems.contains(&target.target.as_str()) {
            problems.push(target.target.as_str());
        }
    }
    if problems.is_empty() {
        return Notice::new("theme", "Theme applied").urgency(Urgency::Low);
    }
    let mut body = problems.iter().take(NAMED).copied().collect::<Vec<_>>().join(", ");
    if problems.len() > NAMED {
        body.push_str(&format!(" and {} more", problems.len() - NAMED));
    }
    Notice::new("theme", "Theme applied with problems")
        .urgency(Urgency::Normal)
        .body(body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Appearance, Selection, TargetResult};

    fn report(targets: &[(&str, &str)]) -> ApplyReport {
        ApplyReport {
            selection: Selection {
                generation: "g".into(),
                appearance: Appearance::Dark,
                follow_system: false,
                targets: Vec::new(),
            },
            targets: targets
                .iter()
                .map(|(target, status)| TargetResult {
                    target: (*target).into(),
                    status: (*status).into(),
                    detail: String::new(),
                })
                .collect(),
        }
    }

    #[test]
    fn all_fine_is_a_low_confirmation() {
        let n = notice(&report(&[("niri", "published"), ("yazi", "ready"), ("kitty", "reload_requested")]));
        assert_eq!((n.kind.as_str(), n.summary.as_str(), n.urgency), ("theme", "Theme applied", Urgency::Low));
        assert!(n.body.is_empty());
    }

    #[test]
    fn needs_selection_is_fine() {
        let n = notice(&report(&[("t3code", "needs_selection"), ("niri", "published")]));
        assert_eq!((n.summary.as_str(), n.urgency), ("Theme applied", Urgency::Low));
    }

    #[test]
    fn one_blocked_target_is_named() {
        let n = notice(&report(&[("niri", "published"), ("zen", "blocked")]));
        assert_eq!((n.summary.as_str(), n.body.as_str(), n.urgency), ("Theme applied with problems", "zen", Urgency::Normal));
    }

    #[test]
    fn several_problems_are_named_in_order() {
        let n = notice(&report(&[("kitty", "reload_failed"), ("niri", "published"), ("zen", "needs_setup")]));
        assert_eq!((n.body.as_str(), n.urgency), ("kitty, zen", Urgency::Normal));
    }

    #[test]
    fn many_problems_are_counted_after_the_first_few() {
        let n = notice(&report(&[
            ("dojo", "blocked"),
            ("niri", "blocked"),
            ("kitty", "blocked"),
            ("zen", "blocked"),
            ("gtk", "reload_failed"),
        ]));
        assert_eq!(n.body, "dojo, niri, kitty and 2 more");
    }

    #[test]
    fn a_target_repeated_is_named_once() {
        let n = notice(&report(&[("zen", "blocked"), ("zen", "needs_setup")]));
        assert_eq!(n.body, "zen");
    }
}
