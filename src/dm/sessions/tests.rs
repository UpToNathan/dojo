use std::os::unix::fs::{PermissionsExt as _, symlink};

use super::*;

const NIRI: &str = "[Desktop Entry]\n\
    Name=Niri (UWSM)\n\
    Name[de]=Niri auf Deutsch\n\
    Comment=Niri Wayland session managed by UWSM\n\
    Exec=uwsm start -N \"Niri\" -D niri -C \"Niri Wayland compositor\" -- /opt/niri-session %U\n\
    TryExec=uwsm\n\
    DesktopNames=niri\n\
    Type=Application\n";

fn rejected(text: &str) -> Rejection {
    parse(text).unwrap_err()
}

#[test]
fn the_niri_entry_parses_with_its_field_codes_removed() {
    assert_eq!(
        parse(NIRI),
        Ok(Parsed {
            name: "Niri (UWSM)".into(),
            exec:
                "uwsm start -N \"Niri\" -D niri -C \"Niri Wayland compositor\" -- /opt/niri-session"
                    .into(),
            desktop_names: vec!["niri".into()],
            try_exec: Some("uwsm".into()),
        })
    );
}

#[test]
fn the_tracked_niri_uwsm_entry_is_offered_with_its_desktop_names() {
    let tracked = include_str!("../../../system-config/wayland-sessions/niri-uwsm.desktop")
        .replace("@USER@", "nathan");
    let parsed = parse(&tracked).unwrap();
    assert_eq!(parsed.name, "Niri (UWSM)");
    assert_eq!(parsed.desktop_names, ["niri"]);
    assert_eq!(parsed.try_exec.as_deref(), Some("uwsm"));
    assert_eq!(
        parsed.exec,
        "uwsm start -N \"Niri\" -D niri -C \"Niri Wayland compositor\" -- /home/nathan/.nix-profile/bin/niri-session"
    );
}

#[test]
fn localized_keys_comments_other_groups_and_unknown_keys_are_ignored() {
    let text = "# a comment\n\n[Desktop Entry]\nName[fr]=Fr\nName=Plain\nExec[de]=nope\nExec=plain\n\
        Icon=whatever\nX-Custom=1\n\n[Desktop Action other]\nName=Other\nExec=other\n";
    let parsed = parse(text).unwrap();
    assert_eq!(
        (parsed.name.as_str(), parsed.exec.as_str()),
        ("Plain", "plain")
    );
    assert_eq!(parsed.desktop_names, Vec::<String>::new());
    assert_eq!(parsed.try_exec, None);
}

#[test]
fn exec_field_codes_are_stripped_and_percent_is_escaped() {
    let exec =
        |value: &str| parse(&format!("[Desktop Entry]\nName=N\nExec={value}\n")).map(|p| p.exec);
    assert_eq!(exec("niri %f %F %u %U %i %c %k"), Ok("niri".into()));
    assert_eq!(exec("niri %d %D %n %N %v %m --x"), Ok("niri --x".into()));
    assert_eq!(exec("echo 100%%"), Ok("echo 100%".into()));
    assert_eq!(exec("a  \t b"), Ok("a b".into()));
    assert_eq!(exec("a\\sb"), Ok("a b".into()));
    assert_eq!(exec("a\\\\b"), Ok("a\\b".into()));
    for bad in [
        "niri %z",
        "niri %",
        "%",
        "%f",
        "   ",
        "niri\\n x",
        "niri\\",
        "niri\\tx",
    ] {
        assert_eq!(exec(bad), Err(Rejection::Invalid("Exec")), "{bad:?}");
    }
}

#[test]
fn desktop_names_are_a_semicolon_list_of_safe_words() {
    let names = |value: &str| {
        parse(&format!(
            "[Desktop Entry]\nName=N\nExec=x\nDesktopNames={value}\n"
        ))
        .map(|p| p.desktop_names)
    };
    assert_eq!(
        names("niri;GNOME;"),
        Ok(vec!["niri".into(), "GNOME".into()])
    );
    assert_eq!(names("niri"), Ok(vec!["niri".into()]));
    assert_eq!(names(""), Ok(vec![]));
    for bad in [
        "a b",
        "a:b",
        "a\\;b",
        "x/y",
        &"n".repeat(65),
        "1;2;3;4;5;6;7;8;9",
    ] {
        assert_eq!(
            names(bad),
            Err(Rejection::Invalid("DesktopNames")),
            "{bad:?}"
        );
    }
}

#[test]
fn hidden_and_unlisted_entries_are_not_offered() {
    for key in ["Hidden", "NoDisplay"] {
        let text = format!("[Desktop Entry]\nName=N\nExec=x\n{key}=true\n");
        assert_eq!(rejected(&text), Rejection::NotListed, "{key}");
        let explicit = format!("[Desktop Entry]\nName=N\nExec=x\n{key}=false\n");
        assert!(parse(&explicit).is_ok(), "{key}=false");
    }
    assert!(matches!(
        rejected("[Desktop Entry]\nName=N\nExec=x\nHidden=maybe\n"),
        Rejection::Invalid(_)
    ));
}

#[test]
fn garbage_is_refused() {
    assert_eq!(rejected(""), Rejection::Malformed);
    assert_eq!(rejected("Name=N\nExec=x\n"), Rejection::Malformed);
    assert_eq!(rejected("[Desktop Entry\nName=N\n"), Rejection::Malformed);
    assert_eq!(
        rejected("[Desktop Entry]\nthis is not a key\n"),
        Rejection::Malformed
    );
    assert_eq!(rejected("[Other]\nName=N\nExec=x\n"), Rejection::Malformed);
    assert_eq!(
        rejected("[Desktop Entry]\nExec=x\n"),
        Rejection::Missing("Name")
    );
    assert_eq!(
        rejected("[Desktop Entry]\nName=N\n"),
        Rejection::Missing("Exec")
    );
    assert_eq!(
        rejected("[Desktop Entry]\nName=\nExec=x\n"),
        Rejection::Invalid("Name")
    );
    assert_eq!(
        rejected("[Desktop Entry]\nName=N\nExec=x\nType=Link\n"),
        Rejection::Invalid("Type")
    );
    assert_eq!(
        rejected("[Desktop Entry]\nName=A\nName=B\nExec=x\n"),
        Rejection::Duplicate("Name")
    );
    assert_eq!(
        rejected("[Desktop Entry]\nName=A\nExec=x\nExec=y\n"),
        Rejection::Duplicate("Exec")
    );
    let long = format!(
        "[Desktop Entry]\nName={}\nExec=x\n",
        "n".repeat(MAX_NAME + 1)
    );
    assert_eq!(rejected(&long), Rejection::Invalid("Name"));
    let long = format!(
        "[Desktop Entry]\nName=N\nExec={}\n",
        "x".repeat(MAX_EXEC + 1)
    );
    assert_eq!(rejected(&long), Rejection::Invalid("Exec"));
    let control = "[Desktop Entry]\nName=N\u{7}\nExec=x\n";
    assert_eq!(rejected(control), Rejection::Invalid("Name"));
    let try_exec = "[Desktop Entry]\nName=N\nExec=x\nTryExec=a\u{1}b\n";
    assert_eq!(rejected(try_exec), Rejection::Invalid("TryExec"));
}

#[test]
fn names_collapse_whitespace_and_empty_try_exec_means_none() {
    let parsed = parse("[Desktop Entry]\nName=  Two   Words \nExec=x\nTryExec=\n").unwrap();
    assert_eq!(parsed.name, "Two Words");
    assert_eq!(parsed.try_exec, None);
}

#[test]
fn try_exec_is_looked_up_in_the_session_path_or_taken_as_absolute() {
    assert_eq!(
        try_exec_candidates("uwsm"),
        [
            PathBuf::from("/usr/local/sbin/uwsm"),
            PathBuf::from("/usr/local/bin/uwsm"),
            PathBuf::from("/usr/bin/uwsm")
        ]
    );
    assert_eq!(
        try_exec_candidates("/opt/x/niri"),
        [PathBuf::from("/opt/x/niri")]
    );
    assert_eq!(try_exec_candidates("../x"), Vec::<PathBuf>::new());
}

fn facts(regular: bool, uid: u32, mode: u32) -> Facts {
    Facts {
        regular,
        directory: !regular,
        uid,
        mode,
    }
}

#[test]
fn an_entry_must_be_a_regular_file_only_root_can_change() {
    assert_eq!(trusted(facts(true, 0, 0o100644), false, 0), Ok(()));
    assert_eq!(
        trusted(facts(true, 0, 0o100664), false, 0),
        Err(Distrust::Writable)
    );
    assert_eq!(
        trusted(facts(true, 0, 0o100646), false, 0),
        Err(Distrust::Writable)
    );
    assert_eq!(
        trusted(facts(true, 1000, 0o100644), false, 0),
        Err(Distrust::Owner)
    );
    assert_eq!(
        trusted(facts(false, 0, 0o040755), false, 0),
        Err(Distrust::NotRegular)
    );
    assert_eq!(trusted(facts(false, 0, 0o040755), true, 0), Ok(()));
    assert_eq!(
        trusted(facts(false, 0, 0o040775), true, 0),
        Err(Distrust::Writable)
    );
    assert_eq!(
        trusted(facts(true, 0, 0o100644), true, 0),
        Err(Distrust::NotDirectory)
    );
}

struct Bench {
    dir: tempfile::TempDir,
    uid: u32,
}

impl Bench {
    fn new() -> Bench {
        let dir = tempfile::Builder::new()
            .prefix("dm-sessions-")
            .tempdir()
            .unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        Bench {
            dir,
            uid: unsafe { libc::geteuid() },
        }
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        fs::write(&path, text).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        path
    }

    fn load(&self) -> (Catalog, Vec<Skipped>) {
        Catalog::load(self.dir.path(), self.uid, |path| {
            path == Path::new("/usr/bin/uwsm")
        })
    }
}

fn id(text: &str) -> SessionId {
    SessionId::parse(text).unwrap()
}

#[test]
fn the_catalog_offers_valid_entries_in_id_order_and_resolves_only_those() {
    let bench = Bench::new();
    bench.write("niri-uwsm.desktop", NIRI);
    bench.write(
        "b-plain.desktop",
        "[Desktop Entry]\nName=Plain\nExec=plain-wm\n",
    );
    bench.write(
        "hidden.desktop",
        "[Desktop Entry]\nName=H\nExec=h\nNoDisplay=true\n",
    );
    bench.write(
        "missing.desktop",
        "[Desktop Entry]\nName=M\nExec=m\nTryExec=not-installed\n",
    );
    bench.write("garbage.desktop", "\u{0}\u{1}");
    bench.write("README", "not an entry");
    bench.write(".hidden-file.desktop", "[Desktop Entry]\nName=D\nExec=d\n");
    let (catalog, skipped) = bench.load();

    let ids: Vec<String> = catalog
        .entries()
        .iter()
        .map(|e| e.id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["b-plain", "niri-uwsm"]);
    let niri = catalog.resolve(&id("niri-uwsm")).unwrap();
    assert_eq!(niri.entry.name, "Niri (UWSM)");
    assert_eq!(catalog.resolve(&id("hidden")), None);
    assert_eq!(catalog.resolve(&id("README")), None);
    assert_eq!(catalog.resolve(&id("never-existed")), None);
    let named: Vec<&str> = skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(
        named,
        [".hidden-file.desktop", "garbage.desktop", "missing.desktop"]
    );

    let start = niri.start(&Vt::new(1).unwrap());
    assert_eq!(start.exec, niri.exec());
    assert_eq!(start.desktop, id("niri-uwsm"));
    assert_eq!(start.desktop_names, ["niri"]);
    assert_eq!((start.vtnr, start.tty.as_str()), (1, "tty1"));
}

#[test]
fn links_writable_files_and_oversized_files_are_refused() {
    let bench = Bench::new();
    let good = "[Desktop Entry]\nName=Good\nExec=good\n";
    let target = bench.write("target.txt", good);
    symlink(&target, bench.dir.path().join("linked.desktop")).unwrap();
    let loose = bench.write("loose.desktop", good);
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o666)).unwrap();
    let group = bench.write("group.desktop", good);
    fs::set_permissions(&group, fs::Permissions::from_mode(0o664)).unwrap();
    bench.write(
        "huge.desktop",
        &format!("{good}# {}\n", "x".repeat(MAX_ENTRY_BYTES as usize)),
    );
    bench.write("ok.desktop", good);
    fs::create_dir(bench.dir.path().join("dir.desktop")).unwrap();

    let (catalog, skipped) = bench.load();
    let ids: Vec<String> = catalog
        .entries()
        .iter()
        .map(|e| e.id.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["ok"]);
    let named: Vec<&str> = skipped.iter().map(|s| s.file.as_str()).collect();
    assert_eq!(
        named,
        [
            "dir.desktop",
            "group.desktop",
            "huge.desktop",
            "linked.desktop",
            "loose.desktop"
        ]
    );
}

#[test]
fn an_entry_owned_by_someone_else_is_refused() {
    let bench = Bench::new();
    bench.write("mine.desktop", "[Desktop Entry]\nName=M\nExec=m\n");
    let (catalog, skipped) = Catalog::load(bench.dir.path(), bench.uid + 1, |_| true);
    assert!(catalog.is_empty());
    assert_eq!(skipped.len(), 1, "the directory itself is not trusted");
}

#[test]
fn a_directory_others_can_write_offers_nothing() {
    let bench = Bench::new();
    bench.write("niri.desktop", "[Desktop Entry]\nName=N\nExec=n\n");
    fs::set_permissions(bench.dir.path(), fs::Permissions::from_mode(0o777)).unwrap();
    let (catalog, skipped) = bench.load();
    assert!(catalog.is_empty());
    assert_eq!(skipped[0].reason, "writable by others");
    let (missing, skipped) = Catalog::load(Path::new("/no/such/dir"), 0, |_| true);
    assert!(missing.is_empty());
    assert_eq!(skipped.len(), 1);
}

#[test]
fn no_more_than_the_protocol_lists_are_offered() {
    let bench = Bench::new();
    for index in 0..MAX_SESSIONS + 5 {
        bench.write(
            &format!("s{index:02}.desktop"),
            "[Desktop Entry]\nName=S\nExec=s\n",
        );
    }
    assert_eq!(bench.load().0.entries().len(), MAX_SESSIONS);
}
