//! S1 model-independent schema/planner policy, translated from the MIT Python package.
//! No model, transport, scheduler, approval authority or desktop effects live here.
use crate::DecodeError as E;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub label: String,
    pub value: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub role: String,
    pub label: String,
    pub value: String,
    pub index: i64,
    pub actions: Vec<String>,
    pub checked: Option<bool>,
    pub element_token: String,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    Fill(usize),
    Check,
    Click,
    Skip,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub element_token: String,
    pub action: Action,
    pub probability: f64,
    pub distribution: Vec<f64>,
}
fn chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}
pub fn render_context(title: &str, element: &Element, placeholder: &str) -> String {
    let state = if element.role == "CheckBox" {
        if element.checked.unwrap_or(false) {
            "checked".into()
        } else {
            "unchecked".into()
        }
    } else {
        format!("value=\"{}\"", chars(&element.value, 48))
    };
    let hint = if placeholder.is_empty() {
        String::new()
    } else {
        format!(" hint=\"{}\"", chars(placeholder, 72))
    };
    format!(
        "TASK fill the form from the document, then submit\nFORM {}\nELEMENT {} \"{}\" {}{}",
        chars(title, 64),
        element.role,
        chars(&element.label, 72),
        state,
        hint
    )
}
pub fn normalize_title(title: &str) -> &str {
    for suffix in [
        " - Google Chrome",
        " - Microsoft Edge",
        " - Mozilla Firefox",
        " - Brave",
        " - Safari",
    ] {
        if let Some(title) = title.strip_suffix(suffix) {
            return title;
        }
    }
    title
}
fn normalized_role(role: &str) -> String {
    role.replace(['_', ' '], "").to_lowercase()
}
pub fn is_actionable_role(role: &str) -> bool {
    matches!(
        normalized_role(role).as_str(),
        "button"
            | "checkbox"
            | "combobox"
            | "edit"
            | "textfield"
            | "axbutton"
            | "axcheckbox"
            | "axcombobox"
            | "axtextfield"
    )
}
fn submit_label(label: &str) -> bool {
    let label = label
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    matches!(label.as_str(), "submit" | "submit form")
}

pub fn render_options(entities: &[Entity]) -> Result<Vec<String>, E> {
    if entities.len() > 253 {
        return Err(E::Limit("entities"));
    }
    let mut options = entities
        .iter()
        .map(|e| format!("fill {}: {}", e.label, e.value))
        .collect::<Vec<_>>();
    if options.iter().any(|s| s.len() > crate::MAX_TEXT_BYTES) {
        return Err(E::Limit("entity text"));
    }
    options.extend(["check", "click", "skip"].map(String::from));
    Ok(options)
}
pub fn decode_option(index: usize, entities: &[Entity]) -> Result<Action, E> {
    if index < entities.len() {
        return Ok(Action::Fill(index));
    }
    match index - entities.len() {
        0 => Ok(Action::Check),
        1 => Ok(Action::Click),
        2 => Ok(Action::Skip),
        _ => Err(E::Invalid("option index")),
    }
}
pub fn encode_option(action: Action, entities: &[Entity]) -> Result<usize, E> {
    match action {
        Action::Fill(i) if i < entities.len() => Ok(i),
        Action::Fill(_) => Err(E::Invalid("entity index")),
        Action::Check => Ok(entities.len()),
        Action::Click => Ok(entities.len() + 1),
        Action::Skip => Ok(entities.len() + 2),
    }
}
fn confidence(p: f64) -> Result<(), E> {
    if p.is_finite() && (0.0..=1.0).contains(&p) {
        Ok(())
    } else {
        Err(E::Invalid("confidence"))
    }
}
/// Require exactly one valid decision for every supplied token. The supplied
/// element data remains authoritative; the model cannot replace role or label.
pub fn validate_plan(
    elements: &[Element],
    entities: &[Entity],
    decisions: &[Decision],
) -> Result<(), E> {
    if elements.len() > 256 || decisions.len() > 256 || entities.len() > 253 {
        return Err(E::Limit("plan"));
    }
    let supplied = elements
        .iter()
        .map(|e| e.element_token.as_str())
        .collect::<HashSet<_>>();
    if supplied.len() != elements.len() || supplied.iter().any(|s| s.is_empty() || s.len() > 1024) {
        return Err(E::Invalid("element identity"));
    }
    let mut decided = HashSet::new();
    for d in decisions {
        if !supplied.contains(d.element_token.as_str()) || !decided.insert(d.element_token.as_str())
        {
            return Err(E::Invalid("foreign or duplicate decision"));
        }
        confidence(d.probability)?;
        encode_option(d.action, entities)?;
        if !d.distribution.is_empty() {
            if d.distribution.len() != entities.len() + 3 {
                return Err(E::Invalid("distribution size"));
            }
            for p in &d.distribution {
                confidence(*p)?;
            }
            if (d.distribution.iter().sum::<f64>() - 1.).abs() > 1e-6 {
                return Err(E::Invalid("distribution sum"));
            }
        }
    }
    if decided.len() != supplied.len() {
        return Err(E::Missing("decision"));
    }
    Ok(())
}
/// Submit selection is only a planning preference. The executor must still obtain
/// durable approval, coordination admission and a fresh snapshot for each effect.
pub fn order_decisions(
    elements: &[Element],
    entities: &[Entity],
    decisions: &[Decision],
    minimum: f64,
    allow_submit: bool,
) -> Result<Vec<Decision>, E> {
    confidence(minimum)?;
    validate_plan(elements, entities, decisions)?;
    let element = |d: &Decision| {
        elements
            .iter()
            .find(|e| e.element_token == d.element_token)
            .expect("validated token")
    };
    let mut keep = decisions
        .iter()
        .filter(|d| d.probability >= minimum && !matches!(d.action, Action::Skip | Action::Click))
        .cloned()
        .collect::<Vec<_>>();
    if allow_submit {
        let submit = decisions
            .iter()
            .filter(|d| d.action == Action::Click && d.probability >= minimum)
            .filter(|d| {
                let e = element(d);
                matches!(normalized_role(&e.role).as_str(), "button" | "axbutton")
                    && submit_label(&e.label)
            })
            .max_by(|a, b| {
                a.probability
                    .total_cmp(&b.probability)
                    .then_with(|| element(b).index.cmp(&element(a).index))
            });
        if let Some(d) = submit {
            keep.push(d.clone());
        }
    }
    keep.sort_by_key(|d| {
        (
            match d.action {
                Action::Fill(_) => 0,
                Action::Check => 1,
                Action::Click => 2,
                Action::Skip => 3,
            },
            element(d).index,
        )
    });
    Ok(keep)
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub snapshot_id: String,
    pub window_id: u64,
    pub pid: u32,
    pub process_start_time: u64,
    pub session_id: String,
    pub runtime_generation: u64,
    pub complete: bool,
    pub elements: Vec<Element>,
}
impl Snapshot {
    fn validate_binding(&self) -> Result<(), E> {
        if !self.complete
            || self.snapshot_id.is_empty()
            || self.snapshot_id.len() > 1024
            || self.window_id == 0
            || self.pid == 0
            || self.process_start_time == 0
            || self.runtime_generation == 0
            || self.session_id.is_empty()
            || self.session_id.len() > 1024
            || self.elements.len() > 256
        {
            return Err(E::Binding);
        }
        Ok(())
    }
    /// Exact token and snapshot binding. Deliberately refuses upstream's index or
    /// role/label fallback after re-observation, which can select another control.
    pub fn resolve(&self, decision: &Decision) -> Result<&Element, E> {
        self.validate_binding()?;
        let mut matches = self
            .elements
            .iter()
            .filter(|e| e.element_token == decision.element_token);
        let element = matches.next().ok_or(E::Binding)?;
        if matches.next().is_some() || element.element_token.is_empty() {
            return Err(E::Binding);
        }
        Ok(element)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Effect {
    Confirmed,
    Refused,
    Unknown,
}
/// After any dispatched mutation, including transport failure, the next Pi phase
/// must re-observe. A successful transport response alone is never confirmation.
pub fn requires_observation_after_dispatch(_effect: Effect) -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Mutation {
    SetValue {
        element_token: String,
        value: String,
    },
    Click {
        element_token: String,
    },
    AlreadySatisfied,
}
/// Construct one mutation from the current snapshot. No index fallback, unknown
/// checkbox toggle, or value replacement without a runtime capability.
pub fn mutation(
    snapshot: &Snapshot,
    decision: &Decision,
    entities: &[Entity],
    supports_set_value: bool,
) -> Result<Mutation, E> {
    confidence(decision.probability)?;
    let e = snapshot.resolve(decision)?;
    match decision.action {
        Action::Fill(i) => {
            if !supports_set_value {
                return Err(E::Unsupported("set_value".into()));
            }
            let value = entities
                .get(i)
                .ok_or(E::Invalid("entity index"))?
                .value
                .clone();
            if value.len() > crate::MAX_TEXT_BYTES {
                return Err(E::Limit("value"));
            }
            Ok(Mutation::SetValue {
                element_token: e.element_token.clone(),
                value,
            })
        }
        Action::Check => {
            if !matches!(normalized_role(&e.role).as_str(), "checkbox" | "axcheckbox") {
                return Err(E::Invalid("check requires checkbox"));
            }
            match e.checked {
                Some(true) => Ok(Mutation::AlreadySatisfied),
                Some(false) => Ok(Mutation::Click {
                    element_token: e.element_token.clone(),
                }),
                None => Err(E::Invalid("checkbox state unknown")),
            }
        }
        Action::Click => Ok(Mutation::Click {
            element_token: e.element_token.clone(),
        }),
        Action::Skip => Ok(Mutation::AlreadySatisfied),
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteExample {
    pub context: String,
    pub options: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ByteBatch {
    pub context_ids: Vec<Vec<u16>>,
    pub context_mask: Vec<Vec<bool>>,
    pub option_ids: Vec<Vec<Vec<u16>>>,
    pub option_token_mask: Vec<Vec<Vec<bool>>>,
    pub option_mask: Vec<Vec<bool>>,
}
/// UTF-8 byte+1 encoding, zero padding and masks shared by S1/Nano text
/// collators. Byte truncation may split a codepoint intentionally, as upstream.
pub fn collate_bytes(
    examples: &[ByteExample],
    context_tokens: usize,
    option_tokens: usize,
) -> Result<ByteBatch, E> {
    if examples.is_empty()
        || examples.len() > 256
        || context_tokens == 0
        || option_tokens == 0
        || context_tokens > 16384
        || option_tokens > 16384
    {
        return Err(E::Limit("collator dimensions"));
    }
    if examples.iter().any(|e| {
        e.options.is_empty()
            || e.options.len() > 256
            || e.context.len() > 65536
            || e.options.iter().any(|s| s.len() > 16384)
    }) {
        return Err(E::Limit("collator examples"));
    }
    let context_width = examples
        .iter()
        .map(|e| e.context.len().min(context_tokens))
        .max()
        .unwrap_or(1)
        .max(1);
    let option_count = examples.iter().map(|e| e.options.len()).max().unwrap_or(1);
    let option_width = examples
        .iter()
        .flat_map(|e| &e.options)
        .map(|s| s.len().min(option_tokens))
        .max()
        .unwrap_or(1)
        .max(1);
    if examples
        .len()
        .checked_mul(option_count)
        .and_then(|n| n.checked_mul(option_width))
        .is_none_or(|n| n > 1_048_576)
    {
        return Err(E::Limit("collator allocation"));
    }
    let ids = |s: &str, width: usize| {
        let mut ids = s
            .bytes()
            .take(width)
            .map(|b| u16::from(b) + 1)
            .collect::<Vec<_>>();
        ids.resize(width, 0);
        ids
    };
    let context_ids = examples
        .iter()
        .map(|e| ids(&e.context, context_width))
        .collect::<Vec<_>>();
    let option_ids = examples
        .iter()
        .map(|e| {
            let mut options = e
                .options
                .iter()
                .map(|s| ids(s, option_width))
                .collect::<Vec<_>>();
            options.resize(option_count, vec![0; option_width]);
            options
        })
        .collect::<Vec<_>>();
    Ok(ByteBatch {
        context_mask: context_ids
            .iter()
            .map(|r| r.iter().map(|id| *id != 0).collect())
            .collect(),
        option_token_mask: option_ids
            .iter()
            .map(|r| {
                r.iter()
                    .map(|o| o.iter().map(|id| *id != 0).collect())
                    .collect()
            })
            .collect(),
        option_mask: examples
            .iter()
            .map(|e| (0..option_count).map(|i| i < e.options.len()).collect())
            .collect(),
        context_ids,
        option_ids,
    })
}

/// Validate the post-effect observation before Pi advances its plan. A transport
/// error uses Unknown and still requires a new snapshot of the exact same target.
pub fn validate_refresh(before: &Snapshot, after: &Snapshot, effect: Effect) -> Result<(), E> {
    before.validate_binding()?;
    after.validate_binding()?;
    if before.snapshot_id == after.snapshot_id
        || after.snapshot_id.is_empty()
        || !after.complete
        || before.window_id != after.window_id
        || before.pid != after.pid
        || before.process_start_time != after.process_start_time
        || before.session_id != after.session_id
        || before.runtime_generation != after.runtime_generation
    {
        return Err(E::Binding);
    }
    if effect != Effect::Confirmed {
        return Err(E::Unsupported(
            "unconfirmed mutation effect; stop and recover".into(),
        ));
    }
    Ok(())
}
