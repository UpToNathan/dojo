//! Pure persisted policy data. Pi owns counters and durable state transitions.
use crate::{DecodeError as E, ModelView, Point, Profile};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroundingKey {
    pub capture_id: String,
    pub capture_digest: String,
    pub session_id: String,
    pub runtime_generation: u64,
    pub geometry_generation: u64,
    pub view_id: String,
    pub profile: Profile,
    pub parser_version: String,
    pub model: String,
    pub description: String,
}
impl GroundingKey {
    pub fn new(view: &ModelView, model: String, description: String) -> Result<Self, E> {
        view.validate(view.profile)?;
        if model.is_empty()
            || model.len() > 1024
            || description.is_empty()
            || description.len() > crate::MAX_TEXT_BYTES
        {
            return Err(E::Invalid("grounding key"));
        }
        Ok(Self {
            capture_id: view.capture_id.clone(),
            capture_digest: view.capture_digest.clone(),
            session_id: view.session_id.clone(),
            runtime_generation: view.runtime_generation,
            geometry_generation: view.geometry_generation,
            view_id: view.view_id.clone(),
            profile: view.profile,
            parser_version: view.parser_version.clone(),
            model,
            description,
        })
    }
}
/// None explicitly retires a previous match. A failed grounder never reuses an
/// earlier point, even when the description is unchanged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundingEntry {
    pub key: GroundingKey,
    pub point: Option<Point>,
}
impl GroundingEntry {
    pub fn lookup(&self, key: &GroundingKey) -> Option<&Point> {
        if self.key == *key {
            self.point.as_ref()
        } else {
            None
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub steps: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_microunits: u64,
}
impl Usage {
    pub fn add(self, other: Self) -> Result<Self, E> {
        Ok(Self {
            steps: self
                .steps
                .checked_add(other.steps)
                .ok_or(E::Limit("steps overflow"))?,
            input_tokens: self
                .input_tokens
                .checked_add(other.input_tokens)
                .ok_or(E::Limit("input token overflow"))?,
            output_tokens: self
                .output_tokens
                .checked_add(other.output_tokens)
                .ok_or(E::Limit("output token overflow"))?,
            cost_microunits: self
                .cost_microunits
                .checked_add(other.cost_microunits)
                .ok_or(E::Limit("cost overflow"))?,
        })
    }
    /// Reserve known cost before a request; account both planner and grounder.
    pub fn admit(self, reservation: Self, limit: Self) -> Result<Self, E> {
        let next = self.add(reservation)?;
        if next.steps > limit.steps
            || next.input_tokens > limit.input_tokens
            || next.output_tokens > limit.output_tokens
            || next.cost_microunits > limit.cost_microunits
        {
            return Err(E::Limit("budget"));
        }
        Ok(next)
    }
}
/// Request projection keeps all message/tool pairing and signatures. The caller
/// replaces images outside these positions without deleting durable entries.
pub fn retained_image_indices(image_count: usize, retain: usize) -> std::ops::Range<usize> {
    image_count.saturating_sub(retain)..image_count
}
