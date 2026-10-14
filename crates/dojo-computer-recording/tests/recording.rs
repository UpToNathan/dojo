use dojo_computer_recording::*;
use serde_json::json;
fn event(time: u64, kind: EventKind, effect: Option<Effect>) -> TraceEvent {
    TraceEvent {
        time_ms: time,
        kind,
        pi_intent_id: Some("intent-1".into()),
        call_id: Some("call-1".into()),
        route: Some("niri".into()),
        observation_id: Some("capture-1".into()),
        effect,
        data: json!({"action": "click", "x": 0, "y": 12}),
    }
}
#[test]
fn roundtrip_redacts_before_retention_and_preserves_action_pairing() {
    let policy = RecordingPolicy {
        secret_values: vec!["secret-value".into()],
        ..Default::default()
    };
    let mut recorder = Recorder::new("session-1", policy.clone()).unwrap();
    let mut intent = event(10, EventKind::Intent, None);
    intent.data = json!({"action": "type", "text": "private", "password": "hunter2",
        "extra": ["Bearer sensitive", {"nested": "secret-value"}], "image_url": "data:image/png;base64,abc"});
    recorder.push(intent).unwrap();
    recorder
        .push(event(12, EventKind::Receipt, Some(Effect::Confirmed)))
        .unwrap();
    let trace = recorder.finish();
    let encoded = serde_json::to_vec(&trace).unwrap();
    let text = String::from_utf8(encoded.clone()).unwrap();
    for secret in [
        "private",
        "hunter2",
        "secret-value",
        "sensitive",
        "base64,abc",
    ] {
        assert!(!text.contains(secret), "{secret}");
    }
    let reread = ingest(&encoded, policy).unwrap();
    assert_eq!(
        serde_json::to_value(trace).unwrap(),
        serde_json::to_value(&reread).unwrap()
    );
    let report = analyze(&reread);
    assert_eq!(report.confirmed, 1);
    assert!(report.unresolved_intents.is_empty());
    assert!(!report.safe_to_replay_effects);
    let atif = export_atif(&reread);
    assert_eq!(atif["schema_version"], "ATIF-v1.8");
    assert_eq!(atif["steps"][0]["tool_calls"][0]["tool_call_id"], "call-1");
}
#[test]
fn unresolved_effect_and_orphan_receipts_are_explicit() {
    let mut recorder = Recorder::new("session", RecordingPolicy::default()).unwrap();
    recorder.push(event(0, EventKind::Intent, None)).unwrap();
    let mut orphan = event(1, EventKind::Receipt, Some(Effect::Unknown));
    orphan.call_id = Some("wrong-call".into());
    recorder.push(orphan).unwrap();
    let report = analyze(&recorder.finish());
    assert_eq!(report.unresolved_intents, ["intent-1"]);
    assert_eq!(report.invalid_receipts, ["intent-1"]);
    assert_eq!(report.unknown, 1);
}
#[test]
fn limits_and_rejected_events_leave_recorder_unchanged() {
    let policy = RecordingPolicy {
        max_events: 1,
        ..Default::default()
    };
    let mut recorder = Recorder::new("session", policy).unwrap();
    assert!(recorder.push(event(0, EventKind::Receipt, None)).is_err());
    recorder.push(event(4, EventKind::Intent, None)).unwrap();
    assert!(
        recorder
            .push(event(5, EventKind::Receipt, Some(Effect::Partial)))
            .is_err()
    );
    assert_eq!(recorder.finish().events().len(), 1);
    let mut recorder = Recorder::new("session", RecordingPolicy::default()).unwrap();
    recorder.push(event(5, EventKind::Intent, None)).unwrap();
    assert!(
        recorder
            .push(event(4, EventKind::Receipt, Some(Effect::Confirmed)))
            .is_err()
    );
}
#[test]
fn images_are_opt_in_bounded_and_digest_checked() {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=").unwrap();
    let mut default = Recorder::new("session", RecordingPolicy::default()).unwrap();
    assert_eq!(default.add_png(&png, false).unwrap(), None);
    let policy = RecordingPolicy {
        retain_images: true,
        ..Default::default()
    };
    let mut recorder = Recorder::new("session", policy.clone()).unwrap();
    assert!(recorder.add_png(&png, false).is_err());
    let digest = recorder.add_png(&png, true).unwrap().unwrap();
    assert_eq!(recorder.add_png(&png, true).unwrap(), Some(digest.clone()));
    let trace = recorder.finish();
    assert_eq!(trace.images().len(), 1);
    assert_eq!(trace.images()[0].width, 1);
    let bytes = serde_json::to_vec(&trace).unwrap();
    assert_eq!(ingest(&bytes, policy.clone()).unwrap().images().len(), 1);
    assert_eq!(
        ingest(&bytes, RecordingPolicy::default())
            .unwrap()
            .images()
            .len(),
        0
    );
    let mut tampered = serde_json::to_value(trace).unwrap();
    tampered["images"][0]["sha256"] = json!("bad");
    assert!(ingest(&serde_json::to_vec(&tampered).unwrap(), policy).is_err());
}
#[test]
fn scalar_reward_and_harbor_estimator_gold_cases() {
    assert_eq!(reward_of(&json!([true, false, null, [1, 0]])), Some(0.5));
    assert_eq!(reward_of(&json!({"reward": 1})), None);
    let results: Vec<_> = [Some(1.0), Some(0.0), None, Some(0.0)]
        .into_iter()
        .map(|reward| TrialResult {
            task: "gold".into(),
            variant: 0,
            reward,
        })
        .collect();
    let metrics = pass_at_k(&results).unwrap();
    assert_eq!(metrics["1"], 0.25);
    assert_eq!(metrics["2"], 0.5);
    assert_eq!(metrics["4"], 1.0);
    assert!(
        pass_at_k(&[TrialResult {
            task: "x".into(),
            variant: 0,
            reward: Some(0.3)
        }])
        .is_none()
    );
}
#[test]
fn deterministic_evaluator_gold_success_failure_unknown() {
    let evaluator = Evaluator::JsonField {
        pointer: "/window/title".into(),
        expected: json!("done"),
    };
    assert_eq!(
        evaluate(&evaluator, Some(&json!({"window": {"title": "done"}}))).reward,
        Some(1.0)
    );
    assert_eq!(
        evaluate(&evaluator, Some(&json!({"window": {"title": "loading"}}))).reward,
        Some(0.0)
    );
    assert_eq!(evaluate(&evaluator, Some(&json!({}))).outcome, "unknown");
    assert_eq!(evaluate(&evaluator, None).reward, None);
}
#[test]
fn transcript_boundary_rejects_hostile_streams_and_renders_real_terminal() {
    let cast = "{\"version\":2,\"width\":8,\"height\":2}\n[0.1,\"o\",\"Hello\"]\n";
    let frame = observe_transcript(cast, 100, &[]).unwrap();
    assert_eq!(frame["schema"], "cua.transcript.frame/1");
    assert!(observe_transcript("{\"version\":2,\"width\":9999999,\"height\":2}", 0, &[]).is_err());
    assert!(observe_transcript(&format!("{cast}[0.01,\"o\",\"stale\"]"), 100, &[]).is_err());
    assert!(observe_transcript(&format!("{cast}bad-event"), 100, &[]).is_err());
}
#[test]
fn nested_secrets_and_key_collisions_are_bounded() {
    assert_eq!(
        redact_value(&json!({"Token": "abc", "safe": "sk-abc"}), &[]).unwrap(),
        json!({"Token": "[redacted]", "safe": "[redacted]"})
    );
    assert!(redact_value(&json!({"secret":1, "[redacted]": 2}), &["secret".into()]).is_err());
    let mut nested = json!(0);
    for _ in 0..40 {
        nested = json!([nested]);
    }
    assert!(redact_value(&nested, &[]).is_err());
}
#[test]
fn terminal_control_limits_prevent_upstream_hang_and_scroll_underflow() {
    for stream in ["\x1b[9999999999999999999I", "\x1b[2S", "\x1b[2M"] {
        let cast = format!(
            "{{\"version\":2,\"width\":8,\"height\":2}}\n{}\n",
            serde_json::to_string(&json!([0, "o", stream])).unwrap()
        );
        assert!(observe_transcript(&cast, 0, &[]).is_err());
    }
    let cast = format!(
        "{{\"version\":2,\"width\":8,\"height\":2}}\n{}\n",
        serde_json::to_string(&json!([0, "o", "\x1b[31mHi\x1b[0m"])).unwrap()
    );
    assert!(observe_transcript(&cast, 0, &[]).is_ok());
}
#[test]
fn known_terminal_secrets_cannot_escape_by_wrapping_across_lines() {
    let cast = "{\"version\":2,\"width\":3,\"height\":3}\n[0.1,\"o\",\"secret\"]\n";
    let frame = observe_transcript(cast, 100, &["secret".into()]).unwrap();
    for line in frame["lines"].as_array().unwrap() {
        assert_eq!(line, "[redacted]");
    }
}
