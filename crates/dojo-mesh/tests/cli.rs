//! The binary on a machine with no Tailscale: a mesh of one, never a failure.

use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static CALLS: AtomicUsize = AtomicUsize::new(0);

fn run(args: &[&str], config: &str) -> (i32, String, String) {
    let dir = std::env::temp_dir().join(format!("dojo-mesh-cli-{}-{}", std::process::id(), CALLS.fetch_add(1, Ordering::Relaxed)));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("mesh.toml");
    std::fs::write(&path, config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
        .arg("--config")
        .arg(&path)
        .args(args)
        .env("PATH", dir.join("no-binaries"))
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    (output.status.code().unwrap(), String::from_utf8_lossy(&output.stdout).into(), String::from_utf8_lossy(&output.stderr).into())
}

#[test]
fn status_without_tailscale_is_a_mesh_of_one() {
    let (code, out, _) = run(&["status"], "membership-tag = \"tag:mesh\"");
    assert_eq!(code, 0);
    assert!(out.starts_with("Mesh of one (tag:mesh)\n"), "{out}");
    assert!(out.contains("tailscale is not installed"), "{out}");
}

#[test]
fn doctor_without_tailscale_warns_and_succeeds() {
    let (code, out, _) = run(&["doctor"], "");
    assert_eq!(code, 0);
    assert!(out.starts_with("warning: The network cannot be read"), "{out}");
}

#[test]
fn json_status_names_the_unavailable_source() {
    let (code, out, _) = run(&["status", "--json"], "");
    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["source"]["kind"], "unavailable");
    assert_eq!(value["members"].as_array().unwrap().len(), 1);
}

#[test]
fn a_bad_config_or_option_exits_2_with_a_reason() {
    let (code, _, err) = run(&["status"], "allow = [\"a\"]\ndeny = [\"a\"]");
    assert_eq!(code, 2);
    assert!(err.contains("both allow and deny"), "{err}");
    let (code, _, err) = run(&["status", "--bogus"], "");
    assert_eq!(code, 2);
    assert!(err.contains("unknown option --bogus"), "{err}");
}
