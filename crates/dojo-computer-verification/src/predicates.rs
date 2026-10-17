// SPDX-License-Identifier: MIT
// Copyright (c) 2026 Cua AI, Inc.
use crate::contract::*;
use serde_json::{Value, json};
use std::collections::HashMap;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ObservationSource {
    #[default]
    Unavailable,
    Native,
    BrowserDom,
}
#[derive(Debug, Clone, Default)]
pub struct ObservationSnapshot {
    pub source: ObservationSource,
    pub window: Option<Value>,
    pub elements: Option<Vec<Value>>,
    pub element_source_trusted: bool,
    /// True only when the platform proved that its structured element walk
    /// completed without truncation. Negative element existence needs this.
    pub elements_complete: bool,
}

pub fn evaluate_predicates(
    expect: &[StatePredicate],
    snapshot: &ObservationSnapshot,
) -> Vec<PredicateOutcome> {
    expect
        .iter()
        .enumerate()
        .map(|(index, predicate)| {
            let (status, reason, observed) = if snapshot.source == ObservationSource::Unavailable {
                (
                    VerificationStatus::Unknown,
                    Some(UnknownReason::UntrustedSource),
                    None,
                )
            } else {
                match (&predicate.window, &predicate.element) {
                    (Some(window), None) => evaluate_window(window, snapshot.window.as_ref()),
                    (None, Some(element)) => evaluate_element(element, snapshot),
                    _ => (
                        VerificationStatus::Unknown,
                        Some(UnknownReason::InvalidPredicate),
                        None,
                    ),
                }
            };
            PredicateOutcome {
                index: index as u64,
                status,
                unknown_reason: reason,
                observed_json: observed.map(|value| bounded_json(&value)),
            }
        })
        .collect()
}

fn evaluate_window(
    predicate: &WindowPredicate,
    window: Option<&Value>,
) -> (VerificationStatus, Option<UnknownReason>, Option<Value>) {
    if predicate.exists.is_none()
        && predicate.bounds.is_none()
        && predicate.title_equals.is_none()
        && predicate.url_equals.is_none()
    {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::InvalidPredicate),
            None,
        );
    }
    let exists = window.is_some();
    if let Some(expected_exists) = predicate.exists {
        if exists != expected_exists {
            return (
                VerificationStatus::Unsatisfied,
                None,
                Some(json!({"exists": exists})),
            );
        }
    }
    let Some(window) = window else {
        return if predicate.exists == Some(false)
            && predicate.bounds.is_none()
            && predicate.title_equals.is_none()
            && predicate.url_equals.is_none()
        {
            (
                VerificationStatus::Satisfied,
                None,
                Some(json!({"exists": false})),
            )
        } else {
            (
                VerificationStatus::Unknown,
                Some(UnknownReason::TargetMissing),
                Some(json!({"exists": false})),
            )
        };
    };
    for (field, expected) in [
        ("title", &predicate.title_equals),
        ("url", &predicate.url_equals),
    ] {
        if let Some(expected) = expected {
            let Some(actual) = window.get(field).and_then(Value::as_str) else {
                return (
                    VerificationStatus::Unknown,
                    Some(UnknownReason::UnsupportedPredicate),
                    Some(json!({"exists":true})),
                );
            };
            if actual != expected {
                return (
                    VerificationStatus::Unsatisfied,
                    None,
                    Some(json!({"exists":true})),
                );
            }
        }
    }
    if let Some(expected) = &predicate.bounds {
        let Some(bounds) = window.get("bounds") else {
            return (
                VerificationStatus::Unknown,
                Some(UnknownReason::UnsupportedPredicate),
                Some(json!({"exists": true})),
            );
        };
        let tolerance = expected.tolerance_px.unwrap_or(1.0);
        let observed = [
            bounds.get("x").and_then(Value::as_f64),
            bounds.get("y").and_then(Value::as_f64),
            bounds.get("width").and_then(Value::as_f64),
            bounds.get("height").and_then(Value::as_f64),
        ];
        if observed.iter().any(Option::is_none) {
            return (
                VerificationStatus::Unknown,
                Some(UnknownReason::UnsupportedPredicate),
                Some(json!({"exists": true, "bounds": bounds})),
            );
        }
        let observed = [
            observed[0].unwrap(),
            observed[1].unwrap(),
            observed[2].unwrap(),
            observed[3].unwrap(),
        ];
        let expected_values = [expected.x, expected.y, expected.width, expected.height];
        if observed
            .iter()
            .zip(expected_values)
            .any(|(actual, expected)| (actual - expected).abs() > tolerance)
        {
            return (
                VerificationStatus::Unsatisfied,
                None,
                Some(json!({"exists": true, "bounds": bounds})),
            );
        }
    }
    (
        VerificationStatus::Satisfied,
        None,
        Some(json!({"exists": true, "bounds": window.get("bounds")})),
    )
}

fn evaluate_element(
    predicate: &ElementPredicate,
    snapshot: &ObservationSnapshot,
) -> (VerificationStatus, Option<UnknownReason>, Option<Value>) {
    if predicate.exists == Some(false)
        && (predicate.value_equals.is_some()
            || predicate.enabled.is_some()
            || predicate.selected.is_some())
    {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::InvalidPredicate),
            None,
        );
    }
    if (predicate.selector.role.is_none() && predicate.selector.label_contains.is_none())
        || predicate
            .selector
            .role
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        || predicate
            .selector
            .label_contains
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
    {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::InvalidPredicate),
            None,
        );
    }
    if snapshot.window.is_none() {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::TargetMissing),
            Some(json!({"window_exists": false})),
        );
    }
    if !snapshot.element_source_trusted {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::UntrustedSource),
            None,
        );
    }
    let Some(elements) = snapshot.elements.as_ref() else {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::ObservationUnavailable),
            None,
        );
    };
    let all_matches: Vec<&Value> = elements
        .iter()
        .filter(|element| selector_matches(&predicate.selector, element))
        .collect();
    // Observations may include display-only rows (static text, read-only
    // values) next to the addressable controls. A control's visible caption
    // is usually such a row, so a selector that matches a control keeps
    // resolving to that control alone; display-only rows are only the target
    // when no addressable element matches.
    let (addressable, display_only): (Vec<&Value>, Vec<&Value>) = all_matches
        .into_iter()
        .partition(|element| element.get("display_only").and_then(Value::as_bool) != Some(true));
    let matches = if addressable.is_empty() {
        display_only
    } else {
        addressable
    };
    if matches.is_empty() {
        let contains_untrusted_region = snapshot.source != ObservationSource::BrowserDom
            && elements.iter().any(|element| {
                element.get("in_web_content").and_then(Value::as_bool) == Some(true)
                    || target_is_in_web_area(element, elements)
            });
        if !snapshot.elements_complete || contains_untrusted_region {
            return (
                VerificationStatus::Unknown,
                Some(if contains_untrusted_region {
                    UnknownReason::UntrustedSource
                } else {
                    UnknownReason::ObservationUnavailable
                }),
                Some(json!({
                    "matches": 0,
                    "elements_complete": snapshot.elements_complete,
                    "contains_untrusted_region": contains_untrusted_region
                })),
            );
        }
        return match predicate.exists {
            Some(false) => (
                VerificationStatus::Satisfied,
                None,
                Some(json!({"matches":0})),
            ),
            Some(true) => (
                VerificationStatus::Unsatisfied,
                None,
                Some(json!({"matches": 0})),
            ),
            _ => (
                VerificationStatus::Unknown,
                Some(UnknownReason::TargetMissing),
                Some(json!({"matches": 0})),
            ),
        };
    }
    let trusted_matches: Vec<&Value> = matches
        .iter()
        .copied()
        .filter(|element| {
            snapshot.source == ObservationSource::BrowserDom
                || !target_is_in_web_area(element, elements)
        })
        .collect();
    if trusted_matches.is_empty() {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::UntrustedSource),
            Some(json!({"matches": matches.len()})),
        );
    }
    if predicate.exists == Some(false) {
        return (
            VerificationStatus::Unsatisfied,
            None,
            Some(json!({"matches":trusted_matches.len()})),
        );
    }
    if !snapshot.elements_complete
        && (predicate.value_equals.is_some()
            || predicate.enabled.is_some()
            || predicate.selected.is_some())
    {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::ObservationUnavailable),
            Some(json!({"matches":trusted_matches.len(),"complete":false})),
        );
    }
    if trusted_matches.len() > 1
        && (predicate.value_equals.is_some()
            || predicate.enabled.is_some()
            || predicate.selected.is_some())
    {
        return (
            VerificationStatus::Unknown,
            Some(UnknownReason::MultiMatch),
            Some(json!({"matches": trusted_matches.len()})),
        );
    }
    if trusted_matches.len() > 1 {
        return (
            VerificationStatus::Satisfied,
            None,
            Some(json!({"matches": trusted_matches.len()})),
        );
    }
    let element = trusted_matches[0];
    for (field, expected) in [
        (
            "value",
            predicate
                .value_equals
                .as_ref()
                .map(|value| Value::String(value.clone())),
        ),
        ("enabled", predicate.enabled.map(Value::Bool)),
        ("selected", predicate.selected.map(Value::Bool)),
    ] {
        let Some(expected) = expected else {
            continue;
        };
        let Some(actual) = element.get(field) else {
            return (
                VerificationStatus::Unknown,
                Some(UnknownReason::UnsupportedPredicate),
                Some(project_element(element)),
            );
        };
        if actual != &expected {
            return (
                VerificationStatus::Unsatisfied,
                None,
                Some(project_element(element)),
            );
        }
    }
    (
        VerificationStatus::Satisfied,
        None,
        Some(project_element(element)),
    )
}

fn selector_matches(selector: &crate::contract::ElementSelector, element: &Value) -> bool {
    if let Some(role) = selector.role.as_deref() {
        if !element
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|actual| normalized_role(actual) == normalized_role(role))
        {
            return false;
        }
    }
    if let Some(label) = selector.label_contains.as_deref() {
        let needle = label.to_lowercase();
        if !element
            .get("label")
            .and_then(Value::as_str)
            .is_some_and(|actual| actual.to_lowercase().contains(&needle))
        {
            return false;
        }
    }
    true
}

fn normalized_role(role: &str) -> String {
    let normalized: String = role
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let normalized = normalized
        .strip_prefix("ax")
        .unwrap_or(&normalized)
        .to_owned();
    match normalized.as_str() {
        "pushbutton" => "button".to_owned(),
        "pagetab" | "tabitem" => "tab".to_owned(),
        _ => normalized,
    }
}

fn target_is_in_web_area(target: &Value, elements: &[Value]) -> bool {
    let by_index: HashMap<u64, &Value> = elements
        .iter()
        .filter_map(|element| {
            element
                .get("element_index")
                .and_then(Value::as_u64)
                .map(|index| (index, element))
        })
        .collect();
    let mut current = Some(target);
    for _ in 0..32 {
        let Some(element) = current else {
            break;
        };
        if element.get("in_web_content").and_then(Value::as_bool) == Some(true) {
            return true;
        }
        if element
            .get("role")
            .and_then(Value::as_str)
            .is_some_and(|role| {
                let normalized = role.to_ascii_lowercase().replace(['_', ' '], "");
                normalized.contains("webarea")
                    || normalized.contains("document")
                    || normalized == "embedded"
            })
        {
            return true;
        }
        current = element
            .get("parent_index")
            .and_then(Value::as_u64)
            .and_then(|index| by_index.get(&index).copied());
    }
    false
}

fn project_element(element: &Value) -> Value {
    let mut projection = serde_json::Map::new();
    for key in [
        "element_index",
        "role",
        "label",
        "value",
        "enabled",
        "selected",
        "display_only",
        "frame",
    ] {
        if let Some(value) = element.get(key) {
            projection.insert(key.to_owned(), value.clone());
        }
    }
    Value::Object(projection)
}

fn bounded_json(value: &Value) -> String {
    let mut encoded = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_owned());
    if encoded.len() > 2_000 {
        let mut boundary = 1_997;
        while !encoded.is_char_boundary(boundary) {
            boundary -= 1;
        }
        encoded.truncate(boundary);
        encoded.push_str("...");
    }
    encoded
}

pub fn aggregate_status(outcomes: &[PredicateOutcome]) -> VerificationStatus {
    if outcomes.is_empty() {
        return VerificationStatus::Unknown;
    }
    if outcomes
        .iter()
        .any(|outcome| outcome.status == VerificationStatus::Unsatisfied)
    {
        VerificationStatus::Unsatisfied
    } else if outcomes
        .iter()
        .any(|outcome| outcome.status == VerificationStatus::Unknown)
    {
        VerificationStatus::Unknown
    } else {
        VerificationStatus::Satisfied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{
        BoundsExpectation, ElementSelector, VerificationStatus, WindowPredicate,
    };

    fn window() -> Value {
        json!({
            "window_id": 7,
            "pid": 42,
            "bounds": {"x": 10.0, "y": 20.0, "width": 300.0, "height": 400.0}
        })
    }

    fn element_predicate(label: &str) -> StatePredicate {
        StatePredicate {
            window: None,
            element: Some(ElementPredicate {
                selector: ElementSelector {
                    role: Some("checkbox".into()),
                    label_contains: Some(label.into()),
                },
                exists: Some(true),
                value_equals: Some("1".into()),
                enabled: None,
                selected: None,
            }),
        }
    }

    #[test]
    fn evaluates_window_bounds_with_tolerance() {
        let predicates = vec![StatePredicate {
            window: Some(WindowPredicate {
                title_equals: None,
                url_equals: None,
                exists: Some(true),
                bounds: Some(BoundsExpectation {
                    x: 10.5,
                    y: 20.0,
                    width: 300.0,
                    height: 400.0,
                    tolerance_px: Some(1.0),
                }),
            }),
            element: None,
        }];
        let outcomes = evaluate_predicates(
            &predicates,
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: None,
                element_source_trusted: false,
                elements_complete: false,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Satisfied);
    }

    #[test]
    fn unique_element_value_is_satisfied() {
        let outcomes = evaluate_predicates(
            &[element_predicate("updates")],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![json!({
                    "element_index": 3,
                    "role": "checkbox",
                    "label": "Automatically check for updates",
                    "value": "1"
                })]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Satisfied);
    }

    fn observed(elements: Vec<Value>) -> ObservationSnapshot {
        ObservationSnapshot {
            source: ObservationSource::Native,
            window: Some(window()),
            elements: Some(elements),
            element_source_trusted: true,
            elements_complete: true,
        }
    }

    fn labelled(label: &str, value_equals: Option<&str>) -> StatePredicate {
        StatePredicate {
            window: None,
            element: Some(ElementPredicate {
                selector: ElementSelector {
                    role: None,
                    label_contains: Some(label.into()),
                },
                exists: Some(true),
                value_equals: value_equals.map(Into::into),
                enabled: None,
                selected: None,
            }),
        }
    }

    #[test]
    fn label_less_display_text_value_is_readable() {
        // #4526: a static text with no title exposes its content via AXValue;
        // the platform reports it as a display-only row labelled by its value.
        let outcomes = evaluate_predicates(
            &[labelled("Saved", Some("Saved"))],
            &observed(vec![json!({
                "role": "AXStaticText",
                "label": "Saved",
                "value": "Saved",
                "display_only": true,
                "parent_index": 0
            })]),
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Satisfied);
    }

    #[test]
    fn addressable_match_wins_over_its_display_only_caption() {
        // The slider's caption is a separate display-only row carrying the
        // same text. It must not turn a value read into a multi-match.
        let outcomes = evaluate_predicates(
            &[labelled("Volume", Some("8"))],
            &observed(vec![
                json!({
                    "role": "AXStaticText",
                    "label": "Volume",
                    "display_only": true
                }),
                json!({
                    "element_index": 1,
                    "role": "AXSlider",
                    "label": "Volume",
                    "value": "8"
                }),
            ]),
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Satisfied);
        assert!(
            outcomes[0]
                .observed_json
                .as_deref()
                .unwrap()
                .contains("\"element_index\":1")
        );
    }

    #[test]
    fn display_only_web_text_stays_untrusted() {
        let outcomes = evaluate_predicates(
            &[labelled("Saved", Some("Saved"))],
            &observed(vec![json!({
                "role": "AXStaticText",
                "label": "Saved",
                "value": "Saved",
                "display_only": true,
                "in_web_content": true
            })]),
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::UntrustedSource)
        );
    }

    #[test]
    fn duplicate_property_target_is_unknown_not_first_match_wins() {
        let element = json!({
            "element_index": 3,
            "role": "checkbox",
            "label": "Updates",
            "value": "1"
        });
        let outcomes = evaluate_predicates(
            &[element_predicate("updates")],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![element.clone(), element]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(outcomes[0].unknown_reason, Some(UnknownReason::MultiMatch));
    }

    #[test]
    fn web_area_readback_is_unknown() {
        let outcomes = evaluate_predicates(
            &[element_predicate("updates")],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![
                    json!({
                        "element_index": 1,
                        "role": "AXWebArea",
                        "label": "Page"
                    }),
                    json!({
                        "element_index": 3,
                        "parent_index": 1,
                        "role": "checkbox",
                        "label": "Updates",
                        "value": "1"
                    }),
                ]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::UntrustedSource)
        );
    }

    #[test]
    fn web_area_existence_is_unknown_even_with_multiple_matches() {
        let mut predicate = element_predicate("updates");
        predicate.element.as_mut().unwrap().value_equals = None;
        let outcomes = evaluate_predicates(
            &[predicate],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![
                    json!({"element_index": 1, "role": "WebArea", "label": "Page"}),
                    json!({
                        "element_index": 2,
                        "parent_index": 1,
                        "role": "checkbox",
                        "label": "Updates"
                    }),
                    json!({
                        "element_index": 3,
                        "parent_index": 1,
                        "role": "checkbox",
                        "label": "Updates"
                    }),
                ]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::UntrustedSource)
        );
    }

    #[test]
    fn normalizes_native_roles_across_platforms() {
        for native_role in ["AXCheckBox", "CheckBox", "check box"] {
            let outcomes = evaluate_predicates(
                &[element_predicate("updates")],
                &ObservationSnapshot {
                    source: ObservationSource::Native,
                    window: Some(window()),
                    elements: Some(vec![json!({
                        "element_index": 3,
                        "role": native_role,
                        "label": "Updates",
                        "value": "1"
                    })]),
                    element_source_trusted: true,
                    elements_complete: true,
                },
            );
            assert_eq!(
                outcomes[0].status,
                VerificationStatus::Satisfied,
                "native role {native_role} should normalize to checkbox"
            );
        }
        assert_eq!(normalized_role("AXButton"), normalized_role("push button"));
        assert_eq!(normalized_role("TabItem"), normalized_role("page tab"));
    }

    #[test]
    fn explicit_windows_and_linux_web_markers_are_untrusted() {
        for native_role in ["Document", "document web"] {
            let outcomes = evaluate_predicates(
                &[element_predicate("updates")],
                &ObservationSnapshot {
                    source: ObservationSource::Native,
                    window: Some(window()),
                    elements: Some(vec![json!({
                        "element_index": 3,
                        "role": native_role,
                        "label": "Updates",
                        "value": "1",
                        "in_web_content": true
                    })]),
                    element_source_trusted: true,
                    elements_complete: true,
                },
            );
            assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
            assert_eq!(
                outcomes[0].unknown_reason,
                Some(UnknownReason::UntrustedSource)
            );
        }
    }

    #[test]
    fn negative_element_existence_is_unknown_for_incomplete_walks() {
        let predicate = StatePredicate {
            window: None,
            element: Some(ElementPredicate {
                selector: ElementSelector {
                    role: Some("button".into()),
                    label_contains: Some("Missing".into()),
                },
                exists: Some(false),
                value_equals: None,
                enabled: None,
                selected: None,
            }),
        };
        let outcomes = evaluate_predicates(
            &[predicate],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![]),
                element_source_trusted: true,
                elements_complete: false,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::ObservationUnavailable)
        );
    }

    #[test]
    fn incomplete_native_walk_cannot_prove_a_unique_control_value() {
        let mut snapshot = observed(vec![
            json!({"element_index": 1, "role": "checkbox", "label": "updates", "value": "1"}),
        ]);
        snapshot.elements_complete = false;
        let outcomes = evaluate_predicates(&[element_predicate("updates")], &snapshot);
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::ObservationUnavailable)
        );
    }

    #[test]
    fn empty_selector_value_is_invalid() {
        let mut predicate = element_predicate("");
        predicate.element.as_mut().unwrap().selector.role = Some(" ".into());
        let outcomes = evaluate_predicates(
            &[predicate],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::InvalidPredicate)
        );
    }

    #[test]
    fn missing_optional_property_is_unknown_not_false() {
        let mut predicate = element_predicate("updates");
        let element = predicate.element.as_mut().unwrap();
        element.value_equals = None;
        element.enabled = Some(true);
        let outcomes = evaluate_predicates(
            &[predicate],
            &ObservationSnapshot {
                source: ObservationSource::Native,
                window: Some(window()),
                elements: Some(vec![json!({
                    "element_index": 3,
                    "role": "checkbox",
                    "label": "Updates"
                })]),
                element_source_trusted: true,
                elements_complete: true,
            },
        );
        assert_eq!(outcomes[0].status, VerificationStatus::Unknown);
        assert_eq!(
            outcomes[0].unknown_reason,
            Some(UnknownReason::UnsupportedPredicate)
        );
    }
}
