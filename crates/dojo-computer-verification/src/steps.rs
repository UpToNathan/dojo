//! Bounded step phases projected into Pi documents. This module dispatches no effects.
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepLimits {
    pub max_steps: u32,
    pub timeout_ms: u64,
}
impl Default for StepLimits {
    fn default() -> Self {
        Self {
            max_steps: 16,
            timeout_ms: 60_000,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    Handled,
    Confirmed,
    Refused,
    Paused,
    Stale,
    Partial,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Refused,
    HumanPause,
    StaleObservation,
    UnknownEffect,
    PartialEffect,
    Cancelled,
    Deadline,
    ClockRewind,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepReceipt {
    pub step: u32,
    pub intent_id: String,
    pub outcome: StepOutcome,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepIntent {
    pub step: u32,
    pub capture_id: String,
    pub intent_id: String,
    pub action_digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepPhase {
    Observe,
    Ready {
        capture_id: String,
        observation_id: String,
    },
    InFlight {
        intent: StepIntent,
    },
    Stopped {
        reason: StopReason,
        ambiguous_step: Option<StepIntent>,
    },
    Finished,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StepRun {
    pub schema_version: u32,
    pub run_id: String,
    pub limits: StepLimits,
    pub started_ms: u64,
    pub last_ms: u64,
    pub phase: StepPhase,
    pub completed_prefix: Vec<StepReceipt>,
    pub seen_observations: Vec<String>,
    pub seen_captures: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StepEvent {
    Observed {
        capture_id: String,
        observation_id: String,
    },
    BeginEffect {
        capture_id: String,
        intent_id: String,
        action_digest: String,
    },
    Receipt {
        intent_id: String,
        outcome: StepOutcome,
    },
    Restart,
    Cancel,
}
fn id(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control),
        "invalid step identifier"
    );
    Ok(())
}
impl StepRun {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            serde_json::to_vec(self)?.len() <= 60 * 1024,
            "serialized step state exceeds budget"
        );
        ensure!(self.schema_version == 1, "unknown step schema");
        id(&self.run_id)?;
        ensure!(
            (1..=64).contains(&self.limits.max_steps)
                && (1..=300_000).contains(&self.limits.timeout_ms),
            "step limits out of range"
        );
        ensure!(self.last_ms >= self.started_ms, "invalid step clock");
        ensure!(
            self.completed_prefix.len() <= self.limits.max_steps as usize,
            "step prefix exceeds budget"
        );
        ensure!(
            self.seen_observations.len() <= 128,
            "observation budget exceeded"
        );
        ensure!(
            self.seen_captures.len() == self.seen_observations.len(),
            "capture/observation pairing mismatch"
        );
        let mut captures = std::collections::BTreeSet::new();
        for capture in &self.seen_captures {
            id(capture)?;
            ensure!(captures.insert(capture), "capture cannot be reused");
        }
        let mut observations = std::collections::BTreeSet::new();
        for observation in &self.seen_observations {
            id(observation)?;
            ensure!(observations.insert(observation), "duplicate observation");
        }
        let mut intents = std::collections::BTreeSet::new();
        for (step, receipt) in self.completed_prefix.iter().enumerate() {
            id(&receipt.intent_id)?;
            ensure!(
                intents.insert(&receipt.intent_id),
                "duplicate effect intent"
            );
            ensure!(
                receipt.step == step as u32
                    && matches!(
                        receipt.outcome,
                        StepOutcome::Handled | StepOutcome::Confirmed
                    ),
                "invalid completed prefix"
            );
        }
        match &self.phase {
            StepPhase::Ready {
                capture_id,
                observation_id,
            } => {
                id(capture_id)?;
                id(observation_id)?;
                ensure!(
                    captures.contains(capture_id) && observations.contains(observation_id),
                    "ready observation was not acquired"
                );
            }
            StepPhase::InFlight {
                intent:
                    StepIntent {
                        step,
                        capture_id,
                        intent_id,
                        action_digest,
                    },
            } => {
                id(capture_id)?;
                id(intent_id)?;
                ensure!(
                    *step as usize == self.completed_prefix.len() && *step < self.limits.max_steps,
                    "invalid active step"
                );
                ensure!(
                    !intents.contains(intent_id),
                    "active intent already completed"
                );
                ensure!(
                    action_digest.len() == 64
                        && action_digest.bytes().all(|b| b.is_ascii_hexdigit()),
                    "invalid intent digest"
                );
            }
            StepPhase::Stopped {
                ambiguous_step: Some(intent),
                ..
            } => {
                ensure!(
                    intent.step as usize == self.completed_prefix.len(),
                    "ambiguous step must follow completed prefix"
                );
                id(&intent.capture_id)?;
                id(&intent.intent_id)?;
                ensure!(
                    captures.contains(&intent.capture_id),
                    "ambiguous capture missing"
                );
                ensure!(
                    intent.action_digest.len() == 64
                        && intent.action_digest.bytes().all(|b| b.is_ascii_hexdigit()),
                    "ambiguous digest invalid"
                );
            }
            StepPhase::Finished => ensure!(
                self.completed_prefix.len() == self.limits.max_steps as usize,
                "unfinished prefix"
            ),
            _ => {}
        }
        if self.completed_prefix.len() == self.limits.max_steps as usize {
            ensure!(
                matches!(self.phase, StepPhase::Finished),
                "exhausted run has no terminal phase"
            );
        }
        Ok(())
    }
    fn stop(&mut self, reason: StopReason, uncertain: bool) {
        let ambiguous_step = match &self.phase {
            StepPhase::InFlight { intent } if uncertain => Some(intent.clone()),
            _ => None,
        };
        self.phase = StepPhase::Stopped {
            reason,
            ambiguous_step,
        };
    }
}
pub fn new_run(run_id: String, limits: StepLimits, now_ms: u64) -> Result<StepRun> {
    let run = StepRun {
        schema_version: 1,
        run_id,
        limits,
        started_ms: now_ms,
        last_ms: now_ms,
        phase: StepPhase::Observe,
        completed_prefix: vec![],
        seen_observations: vec![],
        seen_captures: vec![],
    };
    run.validate()?;
    Ok(run)
}
/// Each transition is finite and independent of transport. Persist its returned state before effects.
pub fn transition(mut run: StepRun, event: StepEvent, now_ms: u64) -> Result<StepRun> {
    run.validate()?;
    if matches!(run.phase, StepPhase::Stopped { .. } | StepPhase::Finished) {
        return Ok(run);
    }
    let uncertain = matches!(run.phase, StepPhase::InFlight { .. });
    if now_ms < run.last_ms {
        run.stop(StopReason::ClockRewind, uncertain);
        return Ok(run);
    }
    run.last_ms = now_ms;
    // A late receipt remains available for operator inspection, but cannot advance the run.
    if now_ms.saturating_sub(run.started_ms) > run.limits.timeout_ms {
        run.stop(StopReason::Deadline, uncertain);
        return Ok(run);
    }
    match event {
        StepEvent::Restart => match run.phase {
            StepPhase::InFlight { .. } => run.stop(StopReason::UnknownEffect, true),
            _ => run.phase = StepPhase::Observe,
        },
        StepEvent::Cancel => run.stop(StopReason::Cancelled, uncertain),
        StepEvent::Observed {
            capture_id,
            observation_id,
        } => {
            id(&capture_id)?;
            id(&observation_id)?;
            ensure!(
                matches!(run.phase, StepPhase::Observe),
                "a new observation cannot overwrite an active step"
            );
            if run.seen_observations.contains(&observation_id)
                || run.seen_captures.contains(&capture_id)
                || run.seen_observations.len() >= 128
            {
                run.stop(StopReason::StaleObservation, false);
            } else {
                run.seen_observations.push(observation_id.clone());
                run.seen_captures.push(capture_id.clone());
                run.phase = StepPhase::Ready {
                    capture_id,
                    observation_id,
                };
            }
        }
        StepEvent::BeginEffect {
            capture_id,
            intent_id,
            action_digest,
        } => {
            id(&capture_id)?;
            id(&intent_id)?;
            ensure!(
                action_digest.len() == 64 && action_digest.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid action digest"
            );
            ensure!(
                !run.completed_prefix
                    .iter()
                    .any(|receipt| receipt.intent_id == intent_id),
                "intent cannot be reused"
            );
            match &run.phase {
                StepPhase::Ready {
                    capture_id: current,
                    ..
                } if current == &capture_id => {
                    run.phase = StepPhase::InFlight {
                        intent: StepIntent {
                            step: run.completed_prefix.len() as u32,
                            capture_id,
                            intent_id,
                            action_digest,
                        },
                    };
                }
                StepPhase::Ready { .. } => run.stop(StopReason::StaleObservation, false),
                _ => bail!("fresh server observation required before each effect"),
            }
        }
        StepEvent::Receipt { intent_id, outcome } => {
            let StepPhase::InFlight {
                intent:
                    StepIntent {
                        step,
                        intent_id: current,
                        ..
                    },
            } = &run.phase
            else {
                bail!("no effect intent is pending");
            };
            ensure!(&intent_id == current, "receipt intent mismatch");
            match outcome {
                StepOutcome::Handled | StepOutcome::Confirmed => {
                    run.completed_prefix.push(StepReceipt {
                        step: *step,
                        intent_id,
                        outcome,
                    });
                    run.phase = if run.completed_prefix.len() == run.limits.max_steps as usize {
                        StepPhase::Finished
                    } else {
                        StepPhase::Observe
                    };
                }
                StepOutcome::Refused => run.stop(StopReason::Refused, false),
                StepOutcome::Paused => run.stop(StopReason::HumanPause, false),
                StepOutcome::Stale => run.stop(StopReason::StaleObservation, false),
                StepOutcome::Partial => run.stop(StopReason::PartialEffect, true),
                StepOutcome::Unknown => run.stop(StopReason::UnknownEffect, true),
            }
        }
    }
    run.validate()?;
    Ok(run)
}
