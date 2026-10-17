//! Action selection copied from CUA native.rs at
//! 88aff7e5424e9ed45b93ded85eb4111472ca5190. See provenance.json.
//! Classification never sends an accessibility request.
use crate::{Node, SelectionParent};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};

/// Activation verbs an AT-SPI action name may carry. Compared against the
/// segment after the last `.`, because GTK4 exposes namespaced action names
/// (`buffer.delete-line`, `clipboard.copy`) while GTK3/Qt expose bare ones
/// (`click`, `activate`).
const ACTIVATION_VERBS: &[&str] = &[
    "click",
    // Chromium exposes this on a static/text node whose clickable target is an
    // ancestor. Dropping it would take away a working path.
    "clickancestor",
    "activate",
    "press",
    "invoke",
    "toggle",
    "open",
    "jump",
    "dodefault",
];

/// Checkbox-only verbs exposed by Chromium's AT-SPI bridge. These are not
/// globally safe activation names: an unrelated widget may advertise a
/// namespaced action ending in `check`, so the role gate is mandatory.
const CHECKBOX_ACTIVATION_VERBS: &[&str] = &["check", "uncheck"];

fn normalized_action_verb(name: &str) -> String {
    name.rsplit('.')
        .next()
        .unwrap_or(name)
        .trim()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Is this action name an activation — the AT-SPI analogue of a click?
///
/// Position is not meaning. A GTK4 text view advertises fifteen actions whose
/// first is `buffer.delete-line`, so actuating "action 0" there deletes a line
/// of the user's document instead of placing a caret.
fn is_activation_action(name: &str) -> bool {
    let verb = normalized_action_verb(name);
    ACTIVATION_VERBS.contains(&verb.as_str())
}

fn is_checkbox_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "check box" | "checkbox"
    )
}

/// Index of the action to actuate, or `None` when the element advertises no
/// activation. `None` must not fall back to index 0: firing an arbitrary
/// action is worse than reporting that there is nothing to fire.
pub fn activation_index(role: &str, actions: &[String]) -> Option<usize> {
    actions.iter().position(|action| {
        is_activation_action(action)
            || (is_checkbox_role(role)
                && CHECKBOX_ACTIVATION_VERBS.contains(&normalized_action_verb(action).as_str()))
    })
}

// Copied literally from the pinned CUA native.rs role policies.
fn is_selectable_item_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "canvas" | "list item" | "table cell" | "cell" | "icon" | "tree item" | "table row" | "row"
    )
}

fn is_option_role(role: &str) -> bool {
    matches!(
        role.trim().to_ascii_lowercase().as_str(),
        "menu item" | "check menu item" | "radio menu item" | "list item" | "option"
    )
}

pub(crate) fn is_direct_selection_role(role: &str) -> bool {
    is_selectable_item_role(role) || is_option_role(role)
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Action {
    Invoke,
    ReplaceText { text: String },
    SetNumeric { value: f64 },
    Select,
}

impl<'de> Deserialize<'de> for Action {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Empty struct variants reject fields that serde ignores on unit variants.
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
        enum Input {
            Invoke {},
            ReplaceText { text: String },
            SetNumeric { value: f64 },
            Select {},
        }
        Ok(match Input::deserialize(deserializer)? {
            Input::Invoke {} => Self::Invoke,
            Input::ReplaceText { text } => Self::ReplaceText { text },
            Input::SetNumeric { value } => Self::SetNumeric { value },
            Input::Select {} => Self::Select,
        })
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum SelectedAction {
    Invoke { index: i32 },
    ReplaceText { text: String },
    SetNumeric { value: f64 },
    Select { parent: SelectionParent },
}

pub fn select(node: &Node, action: &Action) -> Result<SelectedAction> {
    ensure!(!node.redacted, "private accessibility element");
    ensure!(
        node.enabled == Some(true),
        "element is disabled or its state is unavailable"
    );
    ensure!(
        !node
            .states
            .iter()
            .any(|state| state.eq_ignore_ascii_case("defunct")),
        "element is defunct"
    );
    let supports = |interface| node.interfaces.iter().any(|value| value == interface);
    Ok(match action {
        Action::Select => {
            ensure!(
                is_direct_selection_role(&node.role),
                "element is not a selectable item"
            );
            let parent = node
                .selection_parent
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("direct Selection parent is unavailable"))?;
            ensure!(
                parent.bus_owner == node.bus_owner && (0..2048).contains(&parent.index),
                "direct Selection parent is invalid"
            );
            SelectedAction::Select {
                parent: parent.clone(),
            }
        }
        Action::Invoke => {
            ensure!(
                supports("org.a11y.atspi.Action"),
                "element has no Action interface"
            );
            let index = activation_index(&node.role, &node.actions)
                .ok_or_else(|| anyhow::anyhow!("element advertises no activation action"))?;
            SelectedAction::Invoke {
                index: index.try_into()?,
            }
        }
        Action::ReplaceText { text } => {
            ensure!(
                !node.states.iter().any(|state| state == "read-only"),
                "element is read-only"
            );
            ensure!(
                supports("org.a11y.atspi.EditableText"),
                "element has no EditableText interface"
            );
            ensure!(
                text.len() <= 16384 && !text.contains('\0'),
                "replacement text exceeds limits"
            );
            SelectedAction::ReplaceText { text: text.clone() }
        }
        Action::SetNumeric { value } => {
            ensure!(
                !node.states.iter().any(|state| state == "read-only"),
                "element is read-only"
            );
            ensure!(
                supports("org.a11y.atspi.Value"),
                "element has no Value interface"
            );
            let (minimum, maximum) = node
                .numeric_range
                .ok_or_else(|| anyhow::anyhow!("numeric range is unavailable"))?;
            ensure!(
                value.is_finite() && *value >= minimum && *value <= maximum,
                "value is outside the finite advertised range"
            );
            SelectedAction::SetNumeric { value: *value }
        }
    })
}

/// Revalidate the advertised meaning of a snapshot reference. The caller must
/// first find the object in a fresh walk of the same compositor-bound root.
pub fn require_same_element(before: &Node, after: &Node) -> Result<()> {
    ensure!(
        before.bus_owner == after.bus_owner
            && before.object_path == after.object_path
            && before.role == after.role
            && before.name == after.name
            && before.interfaces == after.interfaces
            && before.actions == after.actions
            && before.redacted == after.redacted
            && before.numeric_range == after.numeric_range
            && before.selection_parent == after.selection_parent,
        "accessibility element changed; observe again"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_never_defaults_to_destructive_first_action() {
        assert_eq!(
            activation_index("text", &["buffer.delete-line".into(), "click".into()]),
            Some(1)
        );
        assert_eq!(
            activation_index("text", &["buffer.delete-line".into()]),
            None
        );
        assert_eq!(activation_index("check box", &["check".into()]), Some(0));
        assert_eq!(activation_index("text", &["check".into()]), None);
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    fn element() -> Node {
        Node {
            index: 1,
            parent: Some(0),
            depth: 1,
            bus_owner: ":1.8".into(),
            object_path: "/control".into(),
            role: "spin button".into(),
            name: Some("Quantity".into()),
            description: None,
            text: None,
            value: Some("1".into()),
            redacted: false,
            enabled: Some(true),
            checked: Some(false),
            selected: Some(false),
            states: vec!["enabled".into(), "sensitive".into()],
            actions: vec!["click".into()],
            interfaces: vec![
                "org.a11y.atspi.Action".into(),
                "org.a11y.atspi.EditableText".into(),
                "org.a11y.atspi.Value".into(),
            ],
            numeric_range: Some((0., 10.)),
            selection_parent: None,
            reported_screen_bounds: None,
            bounds_authoritative: false,
        }
    }
    #[test]
    fn typed_selection_checks_live_capabilities_and_finite_ranges() {
        let node = element();
        assert_eq!(
            select(&node, &Action::Invoke).unwrap(),
            SelectedAction::Invoke { index: 0 }
        );
        assert_eq!(
            select(
                &node,
                &Action::ReplaceText {
                    text: "hello".into()
                }
            )
            .unwrap(),
            SelectedAction::ReplaceText {
                text: "hello".into()
            }
        );
        assert_eq!(
            select(&node, &Action::SetNumeric { value: 10. }).unwrap(),
            SelectedAction::SetNumeric { value: 10. }
        );
        for value in [-1., 11., f64::NAN, f64::INFINITY] {
            assert!(select(&node, &Action::SetNumeric { value }).is_err());
        }
        let mut disabled = node.clone();
        disabled.enabled = Some(false);
        assert!(select(&disabled, &Action::Invoke).is_err());
        let mut private = node.clone();
        private.redacted = true;
        assert!(select(&private, &Action::Invoke).is_err());
        let mut readonly = node.clone();
        readonly.states.push("read-only".into());
        assert!(select(&readonly, &Action::ReplaceText { text: "x".into() }).is_err());
        let mut missing = node.clone();
        missing.interfaces.clear();
        assert!(select(&missing, &Action::Invoke).is_err());
        let mut changed = node.clone();
        changed.actions = vec!["delete".into()];
        assert!(require_same_element(&node, &changed).is_err());
    }

    #[test]
    fn select_uses_the_observed_direct_parent_and_rejects_reparenting() {
        let mut node = element();
        node.role = "list item".into();
        node.states = vec!["sensitive".into()];
        node.interfaces.clear();
        node.selection_parent = Some(SelectionParent {
            bus_owner: ":1.8".into(),
            object_path: "/choices".into(),
            index: 3,
        });
        assert_eq!(
            select(&node, &Action::Select).unwrap(),
            SelectedAction::Select {
                parent: SelectionParent {
                    bus_owner: ":1.8".into(),
                    object_path: "/choices".into(),
                    index: 3
                },
            }
        );
        let mut changed = node.clone();
        changed.selection_parent.as_mut().unwrap().object_path = "/other_choices".into();
        assert!(require_same_element(&node, &changed).is_err());
        for index in [-1, 2048, i32::MAX] {
            changed = node.clone();
            changed.selection_parent.as_mut().unwrap().index = index;
            assert!(select(&changed, &Action::Select).is_err());
        }
        changed = node.clone();
        changed.selection_parent.as_mut().unwrap().bus_owner = ":1.9".into();
        assert!(select(&changed, &Action::Select).is_err());
        node.selection_parent = None;
        assert!(select(&node, &Action::Select).is_err());
    }

    #[test]
    fn replacement_text_matches_the_transport_byte_limit() {
        let node = element();
        assert!(
            select(
                &node,
                &Action::ReplaceText {
                    text: "a".repeat(16384)
                }
            )
            .is_ok()
        );
        assert!(
            select(
                &node,
                &Action::ReplaceText {
                    text: "a".repeat(16385)
                }
            )
            .is_err()
        );
        assert!(
            select(
                &node,
                &Action::ReplaceText {
                    text: "é".repeat(8193)
                }
            )
            .is_err()
        );
    }

    #[test]
    fn select_does_not_accept_a_model_supplied_parent_or_index() {
        assert!(serde_json::from_value::<Action>(serde_json::json!({"kind":"select"})).is_ok());
        for supplied in [
            serde_json::json!({"kind":"select", "index":3}),
            serde_json::json!({"kind":"select", "parent":{"bus_owner":":1.8", "object_path":"/choices", "index":3}}),
        ] {
            assert!(serde_json::from_value::<Action>(supplied).is_err());
        }
    }
}
