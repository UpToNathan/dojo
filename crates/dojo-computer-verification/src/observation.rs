//! Trusted-host observation adapters. Tool/model JSON never supplies these identity proofs.
use crate::{ObservationSnapshot, ObservationSource};
use anyhow::{Result, ensure};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetIdentity {
    pub session_id: String,
    pub compositor_epoch: String,
    pub window_id: u64,
    pub client_generation: u64,
    pub pid: i64,
    pub process_start: u64,
}
impl TargetIdentity {
    fn valid(&self) -> bool {
        self.pid > 0
            && self.process_start > 0
            && self.client_generation > 0
            && !self.session_id.is_empty()
            && self.session_id.len() <= 256
            && !self.compositor_epoch.is_empty()
            && self.compositor_epoch.len() <= 256
            && !self
                .session_id
                .chars()
                .chain(self.compositor_epoch.chars())
                .any(char::is_control)
    }
}
#[derive(Clone, Debug, Default)]
pub struct DomCoverage {
    pub walk_complete: bool,
    pub truncated: bool,
    pub missing_frames: u32,
    pub filtered_to_actionable: bool,
    pub document_binding_current: bool,
}
impl DomCoverage {
    pub fn complete(&self) -> bool {
        self.walk_complete
            && !self.truncated
            && self.missing_frames == 0
            && !self.filtered_to_actionable
            && self.document_binding_current
    }
}
#[derive(Clone, Debug, Default)]
pub struct SemanticElement {
    pub index: u64,
    pub role: Option<String>,
    pub label: Option<String>,
    pub value: Option<String>,
    pub enabled: Option<bool>,
    pub selected: Option<bool>,
    pub display_only: bool,
    pub sensitive: bool,
}
#[derive(Clone, Debug)]
pub struct BrowserDomEvidence {
    pub identity: TargetIdentity,
    pub snapshot_id: u64,
    pub document_id: String,
    pub window: Value,
    pub elements: Vec<SemanticElement>,
    pub coverage: DomCoverage,
}
/// Binds one current DOM snapshot to the exact authorized native browser window.
/// Ref-only/action-only lists can prove positive existence, never complete absence.
pub fn browser_observation(
    expected: &TargetIdentity,
    evidence: BrowserDomEvidence,
) -> Result<ObservationSnapshot> {
    ensure!(
        evidence.elements.len() <= 10_000,
        "semantic element budget exceeded"
    );
    ensure!(
        evidence.document_id.len() <= 256,
        "document identifier too large"
    );
    if !expected.valid()
        || !evidence.identity.valid()
        || expected != &evidence.identity
        || evidence.snapshot_id == 0
        || evidence.document_id.is_empty()
        || !evidence.coverage.document_binding_current
    {
        return Ok(ObservationSnapshot::default());
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut elements = Vec::with_capacity(evidence.elements.len());
    for element in evidence.elements {
        ensure!(
            seen.insert(element.index),
            "duplicate semantic element index"
        );
        for value in [&element.role, &element.label, &element.value]
            .into_iter()
            .flatten()
        {
            ensure!(value.len() <= 4096, "semantic text budget exceeded");
        }
        let mut value = json!({"element_index":element.index,"display_only":element.display_only});
        for (key, field) in [("role", element.role), ("label", element.label)] {
            if let Some(field) = field {
                value[key] = Value::String(field);
            }
        }
        if !element.sensitive {
            if let Some(field) = element.value {
                value["value"] = Value::String(field);
            }
        }
        if let Some(field) = element.enabled {
            value["enabled"] = Value::Bool(field);
        }
        if let Some(field) = element.selected {
            value["selected"] = Value::Bool(field);
        }
        elements.push(value);
    }
    ensure!(
        serde_json::to_vec(&evidence.window)?.len() <= 16_384,
        "window evidence budget exceeded"
    );
    ensure!(
        evidence.window.is_object(),
        "window evidence must be an object"
    );
    ensure!(
        evidence.window.get("window_id").and_then(Value::as_u64) == Some(expected.window_id)
            && evidence.window.get("pid").and_then(Value::as_i64) == Some(expected.pid),
        "window evidence identity mismatch"
    );
    Ok(ObservationSnapshot {
        source: ObservationSource::BrowserDom,
        window: Some(evidence.window),
        elements: Some(elements),
        element_source_trusted: true,
        elements_complete: evidence.coverage.complete(),
    })
}
/// An application-wide AT-SPI tree has no exact native-window binding yet.
/// Neither positive nor negative window/element predicates can use it as that proof.
pub fn unbound_accessibility_observation() -> ObservationSnapshot {
    ObservationSnapshot::default()
}
