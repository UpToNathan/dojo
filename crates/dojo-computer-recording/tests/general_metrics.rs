use serde_json::{Value, json};
use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn native_cli_scores_all_general_fixtures_without_host_getters() {
    let cases: Vec<Value> =
        serde_json::from_str(include_str!("../fixtures/general-metrics.json")).unwrap();
    for mut case in cases {
        let reward = case.as_object_mut().unwrap().remove("reward").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_dojo-computer-recording"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        serde_json::to_writer(child.stdin.take().unwrap(), &case).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let result: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(result["reward"].as_f64(), reward.as_f64(), "{case}");
        assert!(!output.stdout.windows(6).any(|part| part == b"secret"));
    }
}

#[test]
fn native_cli_refuses_executable_evaluator_and_unknown_fields() {
    for command in [
        json!({"command":"evaluate_general","metric":{"kind":"run_sqlite3","sql":"DELETE FROM files"},"evidence":null}),
        json!({"command":"evaluate_general","metric":{"kind":"text_equal","expected":"secret-value","unknown":"injected"},"evidence":"secret-value"}),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dojo-computer-recording"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        writeln!(child.stdin.take().unwrap(), "{command}").unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("secret-value"));
    }
}
