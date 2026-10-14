//! Bounded projection of canonical model receipts, grounded in CUA usage callbacks.
//! The caller owns pricing, receipts, deduplication identity and durable budgets.
use crate::Error;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_USAGE_SAMPLES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageSample {
    /// SHA256 of the canonical receipt identity; raw provider IDs are never exported.
    pub receipt_digest: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    /// Caller-reported cost estimate in millionths of USD. None means unavailable.
    pub cost_microusd: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTotals {
    pub samples: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    /// Subsets of input_tokens, never added again to total_tokens.
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub known_cost_microusd: u64,
    pub samples_without_cost: usize,
}

/// All-or-nothing aggregation; duplicate identities refuse rather than double count.
impl UsageTotals {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.samples > MAX_USAGE_SAMPLES
            || self.samples_without_cost > self.samples
            || self.input_tokens.checked_add(self.output_tokens) != Some(self.total_tokens)
            || self
                .cache_read_tokens
                .checked_add(self.cache_write_tokens)
                .is_none_or(|v| v > self.input_tokens)
            || (self.samples == 0 && *self != Self::default())
            || (self.samples_without_cost == self.samples && self.known_cost_microusd != 0)
        {
            return Err(Error::Invalid("usage total invariants"));
        }
        Ok(())
    }
}

pub fn aggregate(samples: &[UsageSample]) -> Result<UsageTotals, Error> {
    if samples.len() > MAX_USAGE_SAMPLES {
        return Err(Error::Limit("usage samples"));
    }
    let mut seen = BTreeSet::new();
    let mut totals = UsageTotals::default();
    fn add(a: &mut u64, b: u64) -> Result<(), Error> {
        *a = a.checked_add(b).ok_or(Error::Limit("usage overflow"))?;
        Ok(())
    }
    for sample in samples {
        if sample.receipt_digest.len() != 64
            || !sample
                .receipt_digest
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || !seen.insert(&sample.receipt_digest)
        {
            return Err(Error::Invalid("usage identity or duplicate receipt"));
        }
        if sample
            .cache_read_tokens
            .checked_add(sample.cache_write_tokens)
            .is_none_or(|tokens| tokens > sample.input_tokens)
        {
            return Err(Error::Invalid("cached token subsets exceed input tokens"));
        }
        add(&mut totals.input_tokens, sample.input_tokens)?;
        add(&mut totals.output_tokens, sample.output_tokens)?;
        add(&mut totals.total_tokens, sample.input_tokens)?;
        add(&mut totals.total_tokens, sample.output_tokens)?;
        add(&mut totals.cache_read_tokens, sample.cache_read_tokens)?;
        add(&mut totals.cache_write_tokens, sample.cache_write_tokens)?;
        if let Some(cost) = sample.cost_microusd {
            add(&mut totals.known_cost_microusd, cost)?;
        } else {
            totals.samples_without_cost += 1;
        }
        totals.samples += 1;
    }
    Ok(totals)
}
