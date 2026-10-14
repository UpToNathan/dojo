use dojo_computer_recording::usage::{UsageSample, UsageTotals, aggregate};
use serde_json::json;
use std::{
    io::Write,
    process::{Command, Stdio},
};

fn sample(digest: char) -> UsageSample {
    UsageSample {
        receipt_digest: digest.to_string().repeat(64),
        input_tokens: 100,
        output_tokens: 20,
        cache_read_tokens: 30,
        cache_write_tokens: 10,
        cost_microusd: Some(125),
    }
}

#[test]
fn callback_fixture_totals_preserve_unknown_cost_and_cache_subsets() {
    let a = sample('a');
    let mut b = sample('b');
    b.cost_microusd = None;
    assert_eq!(
        aggregate(&[a, b]).unwrap(),
        UsageTotals {
            samples: 2,
            input_tokens: 200,
            output_tokens: 40,
            total_tokens: 240,
            cache_read_tokens: 60,
            cache_write_tokens: 20,
            known_cost_microusd: 125,
            samples_without_cost: 1,
        }
    );
    assert_eq!(aggregate(&[]).unwrap(), UsageTotals::default());
}

#[test]
fn duplicate_overflow_malformed_and_inconsistent_usage_refuse() {
    let a = sample('a');
    assert!(aggregate(&[a.clone(), a.clone()]).is_err());
    let mut bad = a.clone();
    bad.receipt_digest = "secret-provider-id".into();
    assert!(aggregate(&[bad]).is_err());
    let mut bad = a.clone();
    bad.input_tokens = 39;
    assert!(aggregate(&[bad]).is_err());
    let mut huge = a;
    huge.input_tokens = u64::MAX;
    assert!(aggregate(&[huge]).is_err());
    assert!(aggregate(&vec![sample('c'); 4097]).is_err());
}

#[test]
fn actual_cli_exposes_typed_usage_without_receipt_identities() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_dojo-computer-recording"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(
            serde_json::to_string(&json!({
                "command": "usage", "samples": [sample('a')]
            }))
            .unwrap()
            .as_bytes(),
        )
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["total_tokens"], 120);
    assert_eq!(value["known_cost_microusd"], 125);
    assert!(value.get("receipt_digest").is_none());
}

#[test]
fn trace_usage_roundtrips_to_atif_and_rejects_forged_totals() {
    use dojo_computer_recording::{Recorder, RecordingPolicy, export_atif, ingest};
    let mut recorder = Recorder::new("usage-fixture", RecordingPolicy::default()).unwrap();
    recorder.set_usage(&[sample('a')]).unwrap();
    assert!(recorder.set_usage(&[sample('a')]).is_err());
    let trace = recorder.finish();
    let bytes = serde_json::to_vec(&trace).unwrap();
    let imported = ingest(&bytes, RecordingPolicy::default()).unwrap();
    assert_eq!(imported.usage().unwrap().total_tokens, 120);
    assert_eq!(
        export_atif(&imported)["final_metrics"]["extra"]["usage"]["known_cost_microusd"],
        125
    );
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    value["usage"]["total_tokens"] = json!(999);
    assert!(
        ingest(
            &serde_json::to_vec(&value).unwrap(),
            RecordingPolicy::default()
        )
        .is_err()
    );
    let old = Recorder::new("old", RecordingPolicy::default())
        .unwrap()
        .finish();
    assert!(serde_json::to_value(old).unwrap().get("usage").is_none());
}
