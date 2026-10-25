//! The operator-only commands through the real binary, with a fake `curl` that records what it
//! was given and answers from a file. No network is touched.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static SITES: AtomicUsize = AtomicUsize::new(0);
const TOKEN: &str = "tskey-api-NOT-A-REAL-TOKEN";

const FAKE_CURL: &str = r#"#!/bin/sh
printf '%s\n' "$@" > "$FAKE_ROOT/curl.args"
cat > "$FAKE_ROOT/curl.stdin"
body=$(sed -n 's/^data-binary = "@\(.*\)"$/\1/p' "$FAKE_ROOT/curl.stdin")
[ -n "$body" ] && cp "$body" "$FAKE_ROOT/curl.body"
[ -f "$FAKE_ROOT/curl.fail" ] && { echo "curl: (6) Could not resolve host" >&2; exit 6; }
cat "$FAKE_ROOT/curl.reply"
"#;

struct Site {
    root: PathBuf,
}

impl Site {
    fn new(config: &str) -> Self {
        let root = std::env::temp_dir().join(format!("dojo-mesh-control-{}-{}", std::process::id(), SITES.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(root.join("bin/curl"), FAKE_CURL).unwrap();
        Command::new("chmod").arg("+x").arg(root.join("bin/curl")).status().unwrap();
        std::fs::write(root.join("mesh.toml"), config).unwrap();
        std::fs::write(root.join("curl.reply"), "HTTP/2 200 \r\nContent-Type: application/json\r\n\r\n{}").unwrap();
        Self { root }
    }

    fn run(&self, origin: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_dojo-mesh"))
            .arg("--config")
            .arg(self.root.join("mesh.toml"))
            .args(args)
            .env_clear()
            .env("PATH", format!("{}:/usr/bin:/bin", self.root.join("bin").display()))
            .env("FAKE_ROOT", &self.root)
            .env("HOME", &self.root)
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_RUNTIME_DIR", &self.root)
            .env("DOJO_TEST_TOKEN", TOKEN)
            .env("DOJO_MESH_ORIGIN", origin)
            .stdin(std::process::Stdio::null())
            .output()
            .unwrap()
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.root.join(name)).unwrap_or_default()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn say(output: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr))
}

const WITH_TOKEN: &str = "membership-tag = \"tag:mesh\"\n[control-plane]\ntailnet = \"example.com\"\napi-token = \"env:DOJO_TEST_TOKEN\"\n";

#[test]
fn validate_sends_plain_json_to_curl_without_the_token_on_its_command_line() {
    let site = Site::new(WITH_TOKEN);
    let policy = site.root.join("policy.hujson");
    std::fs::write(&policy, "{\n  // note\n  \"acls\": [],\n}\n").unwrap();
    let result = site.run("operator", &["policy", "validate", policy.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(0), "{}", say(&result));
    assert!(say(&result).contains("Valid"));

    let args = site.read("curl.args");
    assert_eq!(args.trim(), "--config\n-", "everything else goes through stdin");
    assert!(!args.contains(TOKEN));
    let config = site.read("curl.stdin");
    assert!(config.contains(&format!("header = \"Authorization: Bearer {TOKEN}\"")), "{config}");
    assert!(config.contains("url = \"https://api.tailscale.com/api/v2/tailnet/example.com/acl/validate\""), "{config}");
    assert!(config.contains("proto = \"=https\""));
    let body: serde_json::Value = serde_json::from_str(&site.read("curl.body")).expect("the check gets plain JSON");
    assert_eq!(body["acls"], serde_json::json!([]));
    let staged: Vec<_> = std::fs::read_dir(&site.root).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().starts_with("dojo-mesh-")).collect();
    assert!(staged.is_empty(), "the staged body is removed");
}

#[test]
fn a_rejected_policy_exits_1_with_tailscales_reason() {
    let site = Site::new(WITH_TOKEN);
    std::fs::write(site.root.join("curl.reply"), "HTTP/2 400 \r\n\r\n{\"message\": \"unknown field \\\"nope\\\"\"}").unwrap();
    let policy = site.root.join("p.json");
    std::fs::write(&policy, "{}").unwrap();
    let result = site.run("operator", &["policy", "validate", policy.to_str().unwrap()]);
    assert_eq!(result.status.code(), Some(1));
    assert!(say(&result).contains("unknown field"), "{}", say(&result));
}

#[test]
fn show_prints_the_policy_exactly_as_tailscale_has_it() {
    let site = Site::new(WITH_TOKEN);
    std::fs::write(site.root.join("curl.reply"), "HTTP/2 200 \r\nETag: \"e1\"\r\n\r\n{\n  // keep me\n}\n").unwrap();
    let result = site.run("agent", &["policy", "show"]);
    assert_eq!(result.status.code(), Some(0), "{}", say(&result));
    assert_eq!(String::from_utf8_lossy(&result.stdout), "{\n  // keep me\n}\n");
}

#[test]
fn changing_the_network_needs_a_person_and_never_reaches_curl_otherwise() {
    let site = Site::new(WITH_TOKEN);
    let policy = site.root.join("p.json");
    std::fs::write(&policy, "{}").unwrap();
    for (origin, args) in [
        ("operator", vec!["enroll"]),
        ("agent", vec!["enroll"]),
        ("operator", vec!["policy", "apply", policy.to_str().unwrap()]),
        ("agent", vec!["policy", "apply", policy.to_str().unwrap()]),
    ] {
        let result = site.run(origin, &args);
        assert_eq!(result.status.code(), Some(3), "{origin} {args:?}: {}", say(&result));
    }
    assert_eq!(site.read("curl.args"), "", "curl was never run");
}

#[test]
fn without_a_control_plane_section_the_commands_say_how_to_add_one() {
    let site = Site::new("membership-tag = \"tag:mesh\"\n");
    let result = site.run("operator", &["policy", "show"]);
    assert_eq!(result.status.code(), Some(2));
    assert!(say(&result).contains("[control-plane]"), "{}", say(&result));
}

#[test]
fn a_missing_secret_or_a_broken_curl_is_reported_plainly() {
    let site = Site::new("[control-plane]\napi-token = \"env:DOJO_NOT_SET\"\n");
    let result = site.run("operator", &["policy", "show"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(say(&result).contains("DOJO_NOT_SET is not set"), "{}", say(&result));

    let site = Site::new(WITH_TOKEN);
    std::fs::write(site.root.join("curl.fail"), "").unwrap();
    let result = site.run("operator", &["policy", "show"]);
    assert_eq!(result.status.code(), Some(1));
    assert!(say(&result).contains("Could not resolve host"), "{}", say(&result));
    assert!(!say(&result).contains(TOKEN));
}
