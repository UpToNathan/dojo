//! scripts/update-nix-cache.sh against a fake `dojo-mesh` that prints the offers a test gives it.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static SITES: AtomicUsize = AtomicUsize::new(0);
const KEY_A: &str = "alpha-cache-1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
const KEY_B: &str = "beta-cache-1:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=";
const KEY_A2: &str = "alpha-cache-2:CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC=";

struct Site {
    root: PathBuf,
}

impl Site {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("dojo-nixcache-{}-{}", std::process::id(), SITES.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("mesh"),
            "#!/bin/sh\n[ -f \"$0.fail\" ] && exit 4\ncat \"$0.offers\"\n[ -s \"$0.offers\" ] || exit 1\n",
        )
        .unwrap();
        Command::new("chmod").arg("+x").arg(root.join("mesh")).status().unwrap();
        Self { root }
    }

    fn offers(&self, lines: &[(&str, &str, &str)]) {
        let text: String = lines.iter().map(|(node, key, port)| format!("{node}\t{node}.example.ts.net\tport={port}\tpublic-key={key}\n")).collect();
        std::fs::write(self.root.join("mesh.offers"), text).unwrap();
    }

    fn conf(&self) -> PathBuf {
        self.root.join("dojo-cache.conf")
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new("bash")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/update-nix-cache.sh"))
            .args(args)
            .env("DOJO_MESH", self.root.join("mesh"))
            .env("DOJO_NIX_CACHE_CONF", self.conf())
            .env("DOJO_AS_ROOT", "env")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn text(&self) -> String {
        std::fs::read_to_string(self.conf()).unwrap_or_default()
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

#[test]
fn approving_a_new_cache_writes_it_and_a_second_run_changes_nothing() {
    let site = Site::new();
    site.offers(&[("alpha", KEY_A, "5000")]);
    let first = site.run(&["--yes"]);
    assert!(first.status.success(), "{}", say(&first));
    let text = site.text();
    assert!(text.contains("# peer alpha.example.ts.net 5000 alpha-cache-1:"), "{text}");
    assert!(text.contains("extra-substituters = http://alpha.example.ts.net:5000\n"), "{text}");
    assert!(text.contains(&format!("extra-trusted-public-keys = {KEY_A}\n")), "{text}");
    let second = site.run(&[]);
    assert!(second.status.success(), "no approval is needed when nothing changed: {}", say(&second));
    assert!(say(&second).contains("already up to date"), "{}", say(&second));
    assert_eq!(site.text(), text);
}

#[test]
fn a_new_key_is_never_trusted_without_a_person_saying_so() {
    let site = Site::new();
    site.offers(&[("alpha", KEY_A, "5000")]);
    let refused = site.run(&[]);
    assert_eq!(refused.status.code(), Some(1), "{}", say(&refused));
    assert!(say(&refused).contains("terminal"), "{}", say(&refused));
    assert_eq!(site.text(), "", "nothing was written");

    site.run(&["--yes"]);
    let approved = site.text();
    site.offers(&[("alpha", KEY_A2, "5000")]);
    let swapped = site.run(&[]);
    assert_eq!(swapped.status.code(), Some(1), "a changed key needs approval too");
    assert!(say(&swapped).contains(KEY_A2) && say(&swapped).contains("replaces"), "{}", say(&swapped));
    assert_eq!(site.text(), approved, "the approved key stays in force");
}

#[test]
fn peers_that_stop_answering_are_kept_until_pruned() {
    let site = Site::new();
    site.offers(&[("alpha", KEY_A, "5000"), ("beta", KEY_B, "5001")]);
    assert!(site.run(&["--yes"]).status.success());
    site.offers(&[("beta", KEY_B, "5001")]);
    assert!(site.run(&[]).status.success());
    assert!(site.text().contains("alpha.example.ts.net:5000") && site.text().contains("beta.example.ts.net:5001"));
    assert!(site.run(&["--prune"]).status.success());
    assert!(!site.text().contains("alpha") && site.text().contains("beta"), "{}", site.text());
}

#[test]
fn no_offers_or_an_unreadable_mesh_leave_the_file_alone() {
    let site = Site::new();
    site.offers(&[("alpha", KEY_A, "5000")]);
    site.run(&["--yes"]);
    let before = site.text();
    site.offers(&[]);
    let none = site.run(&[]);
    assert!(none.status.success() && say(&none).contains("leaving"), "{}", say(&none));
    std::fs::write(site.root.join("mesh.fail"), "").unwrap();
    let broken = site.run(&["--yes", "--prune"]);
    assert_eq!(broken.status.code(), Some(1), "{}", say(&broken));
    assert_eq!(site.text(), before);
}

#[test]
fn offers_without_a_usable_port_or_key_are_ignored() {
    let site = Site::new();
    site.offers(&[("alpha", "not-a-key", "5000"), ("beta", KEY_B, "port;rm")]);
    let result = site.run(&["--yes"]);
    assert!(result.status.success(), "{}", say(&result));
    assert!(say(&result).contains("Ignoring alpha") && say(&result).contains("Ignoring beta"), "{}", say(&result));
    assert_eq!(site.text(), "");
}
