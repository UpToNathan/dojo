//! CUA-derived predicates. Observation and session authority remain with Dojo.
pub mod contract;
pub mod observation;
mod predicates;
pub mod steps;
pub use contract::*;
pub use predicates::{
    ObservationSnapshot, ObservationSource, aggregate_status, evaluate_predicates,
};

use anyhow::{Result, ensure};

impl VerifyStateInput {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.pid > 0, "pid must be positive");
        ensure!(
            (1..=8).contains(&self.expect.len()),
            "expected 1 through 8 predicates"
        );
        ensure!(
            self.timeout_ms.unwrap_or(5_000) <= 10_000,
            "timeout exceeds 10000 ms"
        );
        ensure!(
            (1..=5).contains(&self.stable_samples.unwrap_or(2)),
            "stable_samples must be 1 through 5"
        );
        ensure!(
            self.timeout_ms != Some(0) || self.stable_samples.unwrap_or(1) == 1,
            "zero timeout permits one sample"
        );
        ensure!(
            self.include_screenshot != Some(true),
            "use desktop_observe for visual evidence"
        );
        for predicate in &self.expect {
            ensure!(
                predicate.window.is_some() != predicate.element.is_some(),
                "each predicate must select exactly one source"
            );
            if let Some(window) = &predicate.window {
                ensure!(
                    window.exists.is_some()
                        || window.bounds.is_some()
                        || window.title_equals.is_some()
                        || window.url_equals.is_some(),
                    "empty window predicate"
                );
                for value in [&window.title_equals, &window.url_equals]
                    .into_iter()
                    .flatten()
                {
                    ensure!(
                        !value.is_empty() && value.len() <= 4096,
                        "invalid window title/URL predicate"
                    );
                }
                if let Some(bounds) = &window.bounds {
                    ensure!(
                        [bounds.x, bounds.y, bounds.width, bounds.height]
                            .iter()
                            .all(|v| v.is_finite())
                            && bounds.width > 0.
                            && bounds.height > 0.,
                        "invalid bounds"
                    );
                    ensure!(
                        bounds
                            .tolerance_px
                            .is_none_or(|v| v.is_finite() && (0.0..=100.0).contains(&v)),
                        "invalid tolerance"
                    );
                }
            }
            if let Some(element) = &predicate.element {
                ensure!(
                    element.exists != Some(false)
                        || (element.value_equals.is_none()
                            && element.enabled.is_none()
                            && element.selected.is_none()),
                    "negative existence cannot include value/state checks"
                );
                ensure!(
                    element.selector.role.is_some() || element.selector.label_contains.is_some(),
                    "empty selector"
                );
                for value in [
                    &element.selector.role,
                    &element.selector.label_contains,
                    &element.value_equals,
                ]
                .into_iter()
                .flatten()
                {
                    ensure!(
                        !value.trim().is_empty() && value.len() <= 4096,
                        "invalid selector or value"
                    );
                }
            }
        }
        Ok(())
    }

    pub fn required_samples(&self) -> u64 {
        if self.timeout_ms == Some(0) {
            1
        } else {
            self.stable_samples.unwrap_or(2)
        }
    }
}

/// Counts only consecutive satisfied samples. Final truth without stability is unknown.
pub struct Stability {
    required: u64,
    consecutive: u64,
    pub samples: u64,
}
impl Stability {
    pub fn new(input: &VerifyStateInput) -> Result<Self> {
        input.validate()?;
        Ok(Self {
            required: input.required_samples(),
            consecutive: 0,
            samples: 0,
        })
    }
    pub fn sample(&mut self, outcomes: &[PredicateOutcome]) -> bool {
        self.samples += 1;
        self.consecutive = if aggregate_status(outcomes) == VerificationStatus::Satisfied {
            self.consecutive + 1
        } else {
            0
        };
        self.consecutive >= self.required
    }
    pub fn finish(
        &self,
        mut outcomes: Vec<PredicateOutcome>,
        elapsed_ms: u64,
    ) -> VerifyStateOutput {
        let stable = self.consecutive >= self.required;
        if !stable {
            for outcome in &mut outcomes {
                if outcome.status == VerificationStatus::Satisfied {
                    outcome.status = VerificationStatus::Unknown;
                    outcome.unknown_reason = Some(UnknownReason::StabilityUnproven);
                }
            }
        }
        VerifyStateOutput {
            status: aggregate_status(&outcomes),
            stable,
            elapsed_ms,
            samples: self.samples,
            predicates: outcomes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn input() -> VerifyStateInput {
        serde_json::from_value(json!({"pid":42,"window_id":7,"expect":[{"window":{"exists":true}}],"stable_samples":2})).unwrap()
    }
    fn sample(status: VerificationStatus) -> Vec<PredicateOutcome> {
        vec![PredicateOutcome {
            index: 0,
            status,
            unknown_reason: None,
            observed_json: None,
        }]
    }
    #[test]
    fn only_consecutive_truth_satisfies_stability() {
        let mut state = Stability::new(&input()).unwrap();
        assert!(!state.sample(&sample(VerificationStatus::Satisfied)));
        assert!(!state.sample(&sample(VerificationStatus::Unsatisfied)));
        assert!(!state.sample(&sample(VerificationStatus::Satisfied)));
        let unfinished = state.finish(sample(VerificationStatus::Satisfied), 200);
        assert_eq!(unfinished.status, VerificationStatus::Unknown);
        assert_eq!(
            unfinished.predicates[0].unknown_reason,
            Some(UnknownReason::StabilityUnproven)
        );
        assert!(state.sample(&sample(VerificationStatus::Satisfied)));
        assert_eq!(
            state
                .finish(sample(VerificationStatus::Satisfied), 300)
                .status,
            VerificationStatus::Satisfied
        );
    }
    #[test]
    fn polling_bounds_and_invalid_geometry_refuse_before_sampling() {
        for fields in [
            json!({"timeout_ms":10001}),
            json!({"stable_samples":0}),
            json!({"timeout_ms":0,"stable_samples":2}),
            json!({"include_screenshot":true}),
            json!({"expect":[{"window":{"bounds":{"x":0,"y":0,"width":0,"height":10}}}]}),
        ] {
            let mut value = serde_json::to_value(input()).unwrap();
            value
                .as_object_mut()
                .unwrap()
                .extend(fields.as_object().unwrap().clone());
            let input: VerifyStateInput = serde_json::from_value(value).unwrap();
            assert!(Stability::new(&input).is_err());
        }
        assert!(
            serde_json::from_value::<VerifyStateInput>(
                json!({"pid":42,"window_id":7,"expect":[],"session":"forged"})
            )
            .is_err()
        );
    }
}
