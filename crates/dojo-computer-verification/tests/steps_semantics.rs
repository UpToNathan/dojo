use dojo_computer_verification::{observation::*, steps::*, *};
use serde_json::json;
fn run() -> StepRun {
    new_run(
        "run".into(),
        StepLimits {
            max_steps: 2,
            timeout_ms: 100,
        },
        0,
    )
    .unwrap()
}
fn observed(run: StepRun, id: &str, time: u64) -> StepRun {
    transition(
        run,
        StepEvent::Observed {
            capture_id: id.into(),
            observation_id: id.into(),
        },
        time,
    )
    .unwrap()
}
fn effect(run: StepRun, id: &str, intent: &str, time: u64) -> StepRun {
    transition(
        run,
        StepEvent::BeginEffect {
            capture_id: id.into(),
            intent_id: intent.into(),
            action_digest: "a".repeat(64),
        },
        time,
    )
    .unwrap()
}
fn receipt(run: StepRun, intent: &str, outcome: StepOutcome, time: u64) -> StepRun {
    transition(
        run,
        StepEvent::Receipt {
            intent_id: intent.into(),
            outcome,
        },
        time,
    )
    .unwrap()
}
#[test]
fn steps_require_each_new_observation_and_keep_exact_completed_prefix() {
    let mut run = receipt(
        effect(observed(run(), "one", 1), "one", "intent-one", 2),
        "intent-one",
        StepOutcome::Handled,
        3,
    );
    assert!(matches!(run.phase, StepPhase::Observe));
    assert_eq!(run.completed_prefix.len(), 1);
    assert!(
        transition(
            run.clone(),
            StepEvent::BeginEffect {
                capture_id: "one".into(),
                intent_id: "two".into(),
                action_digest: "a".repeat(64)
            },
            4
        )
        .is_err()
    );
    run = receipt(
        effect(observed(run, "two", 4), "two", "intent-two", 5),
        "intent-two",
        StepOutcome::Confirmed,
        6,
    );
    assert!(matches!(run.phase, StepPhase::Finished));
    assert_eq!(run.completed_prefix[0].outcome, StepOutcome::Handled);
    assert_eq!(run.completed_prefix[1].intent_id, "intent-two");
}
#[test]
fn restart_after_intent_or_during_effect_is_ambiguous_and_cannot_replay() {
    let first = receipt(
        effect(observed(run(), "one", 1), "one", "done", 2),
        "done",
        StepOutcome::Handled,
        3,
    );
    let pending = effect(observed(first, "two", 4), "two", "uncertain", 5);
    let persisted: StepRun =
        serde_json::from_slice(&serde_json::to_vec(&pending).unwrap()).unwrap();
    let recovered = transition(persisted, StepEvent::Restart, 6).unwrap();
    let StepPhase::Stopped {
        reason,
        ambiguous_step: Some(intent),
    } = &recovered.phase
    else {
        panic!("missing ambiguity");
    };
    assert_eq!(*reason, StopReason::UnknownEffect);
    assert_eq!(intent.step, 1);
    assert_eq!(intent.intent_id, "uncertain");
    assert_eq!(intent.capture_id, "two");
    assert_eq!(intent.action_digest, "a".repeat(64));
    assert_eq!(recovered.completed_prefix.len(), 1);
    assert_eq!(
        transition(
            recovered.clone(),
            StepEvent::Receipt {
                intent_id: "uncertain".into(),
                outcome: StepOutcome::Confirmed
            },
            7
        )
        .unwrap(),
        recovered
    );
}
#[test]
fn restart_before_intent_retires_ready_capture_without_claiming_effect() {
    let state = transition(observed(run(), "one", 1), StepEvent::Restart, 2).unwrap();
    assert_eq!(state.phase, StepPhase::Observe);
    let reused = transition(
        state,
        StepEvent::Observed {
            capture_id: "one".into(),
            observation_id: "different".into(),
        },
        3,
    )
    .unwrap();
    assert!(matches!(
        reused.phase,
        StepPhase::Stopped {
            reason: StopReason::StaleObservation,
            ambiguous_step: None
        }
    ));
}
#[test]
fn fake_clock_deadline_and_rewind_stop_before_effect_or_preserve_ambiguity() {
    let expired = transition(
        observed(run(), "one", 1),
        StepEvent::BeginEffect {
            capture_id: "one".into(),
            intent_id: "none".into(),
            action_digest: "a".repeat(64),
        },
        101,
    )
    .unwrap();
    assert!(matches!(
        expired.phase,
        StepPhase::Stopped {
            reason: StopReason::Deadline,
            ambiguous_step: None
        }
    ));
    let pending = effect(observed(run(), "one", 1), "one", "unknown", 2);
    let cancelled = transition(pending.clone(), StepEvent::Cancel, 3).unwrap();
    assert!(matches!(
        cancelled.phase,
        StepPhase::Stopped {
            ambiguous_step: Some(_),
            ..
        }
    ));
    let reversed = transition(pending, StepEvent::Restart, 1).unwrap();
    assert!(matches!(
        reversed.phase,
        StepPhase::Stopped {
            reason: StopReason::ClockRewind,
            ambiguous_step: Some(_)
        }
    ));
}
#[test]
fn pause_refusal_stale_partial_and_unknown_never_execute_suffix() {
    for outcome in [
        StepOutcome::Paused,
        StepOutcome::Refused,
        StepOutcome::Stale,
        StepOutcome::Partial,
        StepOutcome::Unknown,
    ] {
        let state = receipt(
            effect(observed(run(), "one", 1), "one", "stop", 2),
            "stop",
            outcome.clone(),
            3,
        );
        let StepPhase::Stopped { ambiguous_step, .. } = &state.phase else {
            panic!("did not stop");
        };
        assert_eq!(
            ambiguous_step.is_some(),
            matches!(outcome, StepOutcome::Partial | StepOutcome::Unknown)
        );
        assert!(state.completed_prefix.is_empty());
        assert_eq!(
            transition(
                state.clone(),
                StepEvent::Observed {
                    capture_id: "new".into(),
                    observation_id: "new".into()
                },
                4
            )
            .unwrap(),
            state
        );
    }
}
fn identity() -> TargetIdentity {
    TargetIdentity {
        session_id: "session".into(),
        compositor_epoch: "epoch".into(),
        window_id: 7,
        client_generation: 1,
        pid: 42,
        process_start: 5,
    }
}
fn evidence() -> BrowserDomEvidence {
    BrowserDomEvidence {
        identity: identity(),
        snapshot_id: 1,
        document_id: "document".into(),
        window: json!({"window_id":7,"pid":42,"title":"Ready","url":"https://example.invalid/ready"}),
        elements: vec![SemanticElement {
            index: 9,
            role: Some("button".into()),
            label: Some("Save".into()),
            enabled: Some(true),
            ..Default::default()
        }],
        coverage: DomCoverage {
            walk_complete: true,
            document_binding_current: true,
            ..Default::default()
        },
    }
}
fn predicate(exists: bool) -> StatePredicate {
    serde_json::from_value(
        json!({"element":{"selector":{"role":"button","label_contains":"Save"},"exists":exists}}),
    )
    .unwrap()
}
#[test]
fn bound_browser_semantics_prove_positive_and_complete_negative() {
    let snapshot = browser_observation(&identity(), evidence()).unwrap();
    assert_eq!(
        evaluate_predicates(&[predicate(true)], &snapshot)[0].status,
        VerificationStatus::Satisfied
    );
    assert_eq!(
        evaluate_predicates(&[predicate(false)], &snapshot)[0].status,
        VerificationStatus::Unsatisfied
    );
    let mut absent = evidence();
    absent.elements.clear();
    let snapshot = browser_observation(&identity(), absent).unwrap();
    assert_eq!(
        evaluate_predicates(&[predicate(false)], &snapshot)[0].status,
        VerificationStatus::Satisfied
    );
    let window: StatePredicate = serde_json::from_value(
        json!({"window":{"title_equals":"Ready","url_equals":"https://example.invalid/ready"}}),
    )
    .unwrap();
    assert_eq!(
        evaluate_predicates(&[window], &snapshot)[0].status,
        VerificationStatus::Satisfied
    );
}
#[test]
fn incomplete_filtered_or_missing_frames_cannot_prove_absence_or_unique_values() {
    for mutate in [0, 1, 2, 3] {
        let mut e = evidence();
        e.elements.clear();
        match mutate {
            0 => e.coverage.truncated = true,
            1 => e.coverage.filtered_to_actionable = true,
            2 => e.coverage.missing_frames = 1,
            _ => e.coverage.walk_complete = false,
        };
        let s = browser_observation(&identity(), e).unwrap();
        assert_eq!(
            evaluate_predicates(&[predicate(false)], &s)[0].status,
            VerificationStatus::Unknown
        );
    }
    let mut e = evidence();
    e.coverage.filtered_to_actionable = true;
    let s = browser_observation(&identity(), e).unwrap();
    let property: StatePredicate = serde_json::from_value(
        json!({"element":{"selector":{"label_contains":"Save"},"enabled":true}}),
    )
    .unwrap();
    assert_eq!(
        evaluate_predicates(&[property], &s)[0].status,
        VerificationStatus::Unknown
    );
    assert_eq!(
        evaluate_predicates(&[predicate(true)], &s)[0].status,
        VerificationStatus::Satisfied
    );
}
#[test]
fn identity_change_and_unbound_accessibility_cannot_manufacture_missing_window_success() {
    let negative: StatePredicate =
        serde_json::from_value(json!({"window":{"exists":false}})).unwrap();
    for kind in [0, 1, 2, 3] {
        let mut e = evidence();
        match kind {
            0 => e.identity.client_generation += 1,
            1 => e.identity.process_start += 1,
            2 => e.identity.session_id = "other".into(),
            _ => e.coverage.document_binding_current = false,
        };
        let s = browser_observation(&identity(), e).unwrap();
        assert_eq!(
            evaluate_predicates(&[negative.clone()], &s)[0].status,
            VerificationStatus::Unknown
        );
    }
    assert_eq!(
        evaluate_predicates(&[negative], &unbound_accessibility_observation())[0].status,
        VerificationStatus::Unknown
    );
}
#[test]
fn sensitive_values_missing_fields_and_duplicate_targets_are_unknown() {
    let value: StatePredicate = serde_json::from_value(
        json!({"element":{"selector":{"label_contains":"Save"},"value_equals":"secret"}}),
    )
    .unwrap();
    let mut e = evidence();
    e.elements[0].value = Some("secret".into());
    e.elements[0].sensitive = true;
    let s = browser_observation(&identity(), e).unwrap();
    let result = evaluate_predicates(&[value.clone()], &s);
    assert_eq!(result[0].status, VerificationStatus::Unknown);
    assert!(!result[0].observed_json.as_ref().unwrap().contains("secret"));
    let mut e = evidence();
    let mut duplicate = e.elements[0].clone();
    duplicate.index = 10;
    e.elements.push(duplicate);
    let s = browser_observation(&identity(), e).unwrap();
    assert_eq!(
        evaluate_predicates(&[value], &s)[0].status,
        VerificationStatus::Unknown
    );
    assert_eq!(aggregate_status(&[]), VerificationStatus::Unknown);
}
