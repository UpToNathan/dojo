use dojo_computer_recording::{RecordingPolicy, benchmark::*, processors::*};
use serde_json::json;
fn task(id: &str, answers: Option<Vec<Answer>>) -> DatasetTask {
    DatasetTask {
        task_id: id.into(),
        image_sha256: "a".repeat(64),
        width: 100,
        height: 100,
        coordinate_space: CoordinateSpace::ImagePixels,
        target: Target::Box {
            xyxy: [10.0, 20.0, 30.0, 40.0],
        },
        answers,
        cancelled_after: None,
    }
}
#[test]
fn screenspot_protocol_agrees_with_upstream_gold_scores() {
    // Gold cases follow ScreenSpotProAdapter.evaluate and grounding.in_xyxy literally.
    let cases = [
        (vec![Answer::Click { x: 10.0, y: 20.0 }], 1.0),
        (vec![Answer::DoubleClick { x: 30.0, y: 40.0 }], 1.0),
        (vec![Answer::Click { x: 9.0, y: 20.0 }], 0.0),
        (vec![Answer::Move { x: 20.0, y: 30.0 }], 0.0),
        (
            vec![
                Answer::Move { x: 0.0, y: 0.0 },
                Answer::RightClick { x: 20.0, y: 30.0 },
            ],
            1.0,
        ),
        (
            vec![
                Answer::Click { x: 20.0, y: 30.0 },
                Answer::Click { x: 20.0, y: 30.0 },
            ],
            0.0,
        ),
        (vec![], 0.0),
    ];
    for (answers, expected) in cases {
        let result = run_episode(
            &task("synthetic-screenspot", Some(answers)),
            &EpisodeLimits::default(),
            RecordingPolicy::default(),
        )
        .unwrap();
        assert_eq!(result.reward, Some(expected));
        assert_eq!(
            result.phases.iter().map(|p| p.name).collect::<Vec<_>>(),
            ["setup", "answer", "evaluate", "cleanup"]
        );
        assert!(result.phases.last().unwrap().completed);
    }
}
#[test]
fn episode_budget_cancellation_and_missing_evidence_are_unknown() {
    let mut cancelled = task(
        "cancelled",
        Some(vec![
            Answer::Click { x: 20.0, y: 30.0 },
            Answer::Click { x: 20.0, y: 30.0 },
        ]),
    );
    cancelled.cancelled_after = Some(1);
    let limits = EpisodeLimits {
        max_steps: 1,
        ..Default::default()
    };
    let result = run_episode(&cancelled, &limits, RecordingPolicy::default()).unwrap();
    assert_eq!(result.reward, None);
    assert_eq!(result.completed_steps, 1);
    assert_eq!(result.outcome, "unknown");
    let report = run_tasks(
        &[
            task("unknown", None),
            task("pass", Some(vec![Answer::MiddleClick { x: 20.0, y: 30.0 }])),
            task("fail", Some(vec![])),
        ],
        &EpisodeLimits::default(),
        RecordingPolicy::default(),
    )
    .unwrap();
    assert_eq!(report.completed, 2);
    assert_eq!(report.failed, 1);
    assert_eq!(report.unknown, 1);
    assert_eq!(report.avg_reward, Some(0.5));
}
#[test]
fn polygon_edges_and_infeasible_targets_follow_cua_rules() {
    let polygon = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
    assert!(in_polygon([10.0, 5.0], &polygon));
    assert!(!in_polygon([11.0, 5.0], &polygon));
    assert!(in_xyxy([5.0, 5.0], [10.0, 10.0, 0.0, 0.0]));
    assert_eq!(
        grounding_reward(&Target::Infeasible, &[Answer::ReportInfeasible]),
        1.0
    );
    assert_eq!(
        grounding_reward(
            &Target::Infeasible,
            &[Answer::Click { x: 0.0, y: 0.0 }, Answer::ReportInfeasible]
        ),
        0.0
    );
}
fn input(html: &str) -> ProcessorInput {
    ProcessorInput {
        annotated_html: html.into(),
        width: 100,
        height: 80,
        os_type: "linux".into(),
        image_sha256: "b".repeat(64),
        capture_consent: true,
        secret_values: vec![],
        rgb_base64: None,
        action_coordinate_space: None,
    }
}
const HTML: &str = r#"<button aria-label="Save" data-instruction="save document" data-bbox-center-hit="true" data-bbox-x="10" data-bbox-y="20" data-bbox-width="20" data-bbox-height="16">Save</button><button aria-label="Save" data-bbox-center-hit="true" data-bbox-x="10" data-bbox-y="20" data-bbox-width="20" data-bbox-height="16">Save</button><button aria-label="Hidden" data-bbox-center-hit="false" data-bbox-x="0" data-bbox-y="0" data-bbox-width="10" data-bbox-height="10"></button>"#;
#[test]
fn gui_r1_annotated_html_matches_upstream_normalization_and_dedup_gold() {
    let rows = gui_r1(&input(HTML)).unwrap();
    assert_eq!(rows.len(), 1);
    let value = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(value["gt_bbox"], json!([0.1, 0.25, 0.3, 0.45]));
    assert_eq!(value["instruction"], "click the UI element Save");
    assert_eq!(value["gt_action"], "click");
    assert_eq!(value["resolution"], "100x80");
    let fallback = gui_r1(&input(r#"<div data-instruction="Go" data-bbox-center-hit="true" data-bbox-x="-10" data-bbox-y="20" data-bbox-width="20" data-bbox-height="16"></div>"#)).unwrap();
    assert_eq!(fallback[0].gt_bbox, [0.0, 0.25, 0.1, 0.45]);
}
#[test]
fn aguvis_augmentation_is_seeded_and_declares_coordinates() {
    let value = input(HTML);
    let first = serde_json::to_value(aguvis(&value, 42, 5).unwrap()).unwrap();
    let second = serde_json::to_value(aguvis(&value, 42, 5).unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first["coordinate_space"], "normalized_0_1");
    let pairs = first["texts"].as_array().unwrap();
    assert!(!pairs.is_empty());
    for pair in pairs {
        assert_eq!(pair["point"], json!([0.2, 0.35]));
    }
    assert_eq!(aguvis(&value, 42, 0).unwrap().texts.len(), 0);
}
#[test]
fn processor_requires_capture_consent_and_filters_known_label_secrets() {
    let mut value = input(HTML);
    value.capture_consent = false;
    assert!(gui_r1(&value).is_err());
    value.capture_consent = true;
    value.secret_values = vec!["Save".into()];
    let rows = serde_json::to_string(&gui_r1(&value).unwrap()).unwrap();
    assert!(!rows.contains("Save"));
    value.annotated_html = HTML.replace("data-bbox-x=\"10\"", "data-bbox-x=\"NaN\"");
    assert!(gui_r1(&value).unwrap().is_empty());
}
#[test]
fn aguvis_text_border_shrinking_gold_rgb_image() {
    use base64::Engine;
    let mut value = input(
        r#"<span data-bbox-center-hit="true" data-bbox-x="1" data-bbox-y="1" data-bbox-width="6" data-bbox-height="6">Target</span>"#,
    );
    value.width = 10;
    value.height = 10;
    let mut pixels = vec![0; 10 * 10 * 3];
    // Adjacent equal borders leave a 3x3 box at x=2,y=2.
    let center = (3 * 10 + 3) * 3;
    pixels[center..center + 3].fill(255);
    value.rgb_base64 = Some(base64::engine::general_purpose::STANDARD.encode(pixels));
    let result = aguvis(&value, 42, 5).unwrap();
    assert!(!result.texts.is_empty());
    for pair in result.texts {
        assert_eq!(pair.point, [0.35, 0.35]);
    }
}
#[test]
fn predefined_actions_require_explicit_spaces_and_reject_executable_suffixes() {
    let mut value =
        input(r#"<div data-actions='[{"user":"select","assistant":"click(x=20,y=28)"}]'></div>"#);
    assert!(aguvis(&value, 3, 5).is_err());
    value.action_coordinate_space = Some(CoordinateSpace::ImagePixels);
    let result = aguvis(&value, 3, 5).unwrap();
    assert_eq!(result.texts.len(), 1);
    assert_eq!(result.texts[0].point, [0.2, 0.35]);
    value.annotated_html = value
        .annotated_html
        .replace("click(x=20,y=28)", "click(x=20,y=28);exec('evil')");
    assert!(aguvis(&value, 3, 5).is_err());
}
#[test]
fn offline_benchmark_cli_runs_fixture_task_set_and_emits_scores() {
    use std::io::Write;
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_dojo-computer-recording"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(include_bytes!("fixtures/offline-benchmark.json"))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["avg_reward"], 0.5);
    assert_eq!(result["completed"], 2);
    assert_eq!(result["unknown"], 1);
    assert_eq!(result["results"][0]["reward"], 1.0);
    assert_eq!(result["results"][1]["reward"], 0.0);
    assert_eq!(result["results"][2]["reward"], serde_json::Value::Null);
}
