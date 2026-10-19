use std::fs;
use std::os::unix::fs::symlink;

use super::*;
use crate::session::settings::SessionSettings;
use crate::session::snapshot::{Line, Palette, Wallpaper};

fn login(text: &str) -> Login {
    Login::parse(text).unwrap()
}

fn palette_bytes() -> Vec<u8> {
    let preset = &crate::theme::presets().unwrap()[0];
    serde_json::to_vec_pretty(&preset.palette).unwrap()
}

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::RgbaImage::new(48, 27)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

fn collected(look: Look, rejected: Vec<File>) -> Decoded {
    Decoded {
        look,
        rejected: rejected
            .into_iter()
            .map(|file| snapshot::Problem {
                file,
                reason: "bad".into(),
            })
            .collect(),
    }
}

fn full_look() -> Look {
    Look {
        login: login("alyssa"),
        full_name: Line::parse("Alyssa Park"),
        palette: Some(Palette::parse(palette_bytes()).unwrap()),
        wallpaper: Some(Wallpaper::from_png(&png_bytes()).unwrap()),
        settings: Some(SessionSettings::default()),
        last_greeting: Line::parse("Welcome back"),
    }
}

#[test]
fn pieces_cross_the_pipe_without_the_identity() {
    let bytes = encode_pieces(&collected(full_look(), vec![]));
    let pieces = decode_pieces(&bytes).unwrap();
    let files: Vec<File> = pieces.iter().map(|(file, _)| *file).collect();
    assert_eq!(
        files,
        [
            File::Palette,
            File::Wallpaper,
            File::Settings,
            File::Greeting
        ]
    );
    let look = assemble(login("alyssa"), Some("Alyssa Park,,,"), &pieces, None);
    assert_eq!(look, full_look());
}

#[test]
fn the_full_name_comes_from_passwd_not_from_the_child() {
    let look = assemble(login("alyssa"), None, &[], None);
    assert_eq!(look.full_name, None);
    let look = assemble(login("alyssa"), Some("Alyssa, Room 4"), &[], None);
    assert_eq!(look.full_name, Line::parse("Alyssa"));
}

#[test]
fn a_piece_the_parent_rejects_keeps_the_previous_one() {
    let pieces = vec![
        (File::Palette, Piece::Present(b"not a palette".to_vec())),
        (File::Wallpaper, Piece::Rejected),
        (File::Greeting, Piece::Present(b"Hello again\n".to_vec())),
    ];
    let previous = full_look();
    let look = assemble(login("alyssa"), None, &pieces, Some(&previous));
    assert_eq!(
        look.palette, previous.palette,
        "invalid bytes keep the old palette"
    );
    assert_eq!(
        look.wallpaper, previous.wallpaper,
        "a rejected file keeps the old wallpaper"
    );
    assert_eq!(look.settings, None, "an absent file is gone");
    assert_eq!(look.last_greeting, Line::parse("Hello again"));
}

#[test]
fn the_child_cannot_smuggle_an_identity_a_repeat_or_an_oversize_piece() {
    let record = |file: u8, status: u8, body: &[u8]| {
        let mut out = vec![file, status];
        out.extend((body.len() as u32).to_le_bytes());
        out.extend(body);
        out
    };
    assert!(decode_pieces(&record(0, PRESENT, b"{}")).is_some());
    assert_eq!(decode_pieces(b""), Some(vec![]));
    let twice = [record(0, PRESENT, b"{}"), record(0, PRESENT, b"{}")].concat();
    assert_eq!(decode_pieces(&twice), None, "a repeated file");
    assert_eq!(
        decode_pieces(&record(3, PRESENT, b"Mallory")),
        None,
        "identity"
    );
    assert_eq!(
        decode_pieces(&record(9, PRESENT, b"x")),
        None,
        "unknown file"
    );
    assert_eq!(decode_pieces(&record(0, 7, b"x")), None, "unknown status");
    assert_eq!(
        decode_pieces(&record(0, REJECTED, b"x")),
        None,
        "a rejection with bytes"
    );
    let big = vec![0u8; (snapshot::PALETTE_CAP + 1) as usize];
    assert_eq!(
        decode_pieces(&record(0, PRESENT, &big)),
        None,
        "over the cap"
    );
    let mut cut = record(0, PRESENT, b"abcdef");
    cut.truncate(cut.len() - 1);
    assert_eq!(decode_pieces(&cut), None, "a cut record");
    assert_eq!(decode_pieces(&[0, 1, 2]), None, "a cut header");
}

#[test]
fn decoding_is_total_over_mutated_input() {
    let valid = encode_pieces(&collected(full_look(), vec![File::Settings]));
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    for _ in 0..2_000 {
        let mut mutated = valid.clone();
        for _ in 0..3 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let index = (seed % mutated.len() as u64) as usize;
            mutated[index] = (seed >> 32) as u8;
        }
        if let Some(pieces) = decode_pieces(&mutated) {
            let _ = assemble(login("alyssa"), None, &pieces, None);
        }
    }
}

fn own_account(home: &Path) -> Option<Account> {
    let uid = unsafe { libc::geteuid() };
    if uid == 0 {
        return None;
    }
    Some(Account {
        login: login("tester"),
        name: c"tester".into(),
        uid,
        gid: unsafe { libc::getegid() },
        home: home.to_owned(),
        shell: "/bin/sh".into(),
    })
}

#[cfg(feature = "fake-pam-stack")]
#[test]
fn a_child_reads_the_home_and_the_parent_stores_what_it_sent() {
    let home = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let Some(account) = own_account(home.path()) else {
        return;
    };
    let themes = home.path().join(".local/state/dojo/themes");
    fs::create_dir_all(&themes).unwrap();
    let generation = themes.join("generation-1");
    fs::create_dir(&generation).unwrap();
    fs::write(generation.join("palette.json"), palette_bytes()).unwrap();
    symlink(&generation, themes.join("current")).unwrap();
    let snapshots = Snapshots::new(state.path().to_owned(), Some(unsafe { libc::getegid() }));
    snapshots.prepare().unwrap();

    snapshots.refresh(&account, Some("Tess Ter,,,")).unwrap();
    let stored = snapshot::load(state.path(), "tester").unwrap();
    assert!(stored.palette.is_some());
    assert_eq!(stored.full_name, Line::parse("Tess Ter"));
    assert_eq!(stored.wallpaper, None);

    fs::write(generation.join("palette.json"), b"garbage").unwrap();
    snapshots.refresh(&account, Some("Tess Ter,,,")).unwrap();
    let kept = snapshot::load(state.path(), "tester").unwrap();
    assert_eq!(
        kept.palette, stored.palette,
        "an invalid palette keeps the last good one"
    );
}

#[cfg(feature = "fake-pam-stack")]
#[test]
fn a_home_that_cannot_be_opened_leaves_the_previous_snapshot_alone() {
    let state = tempfile::tempdir().unwrap();
    let Some(account) = own_account(Path::new("/no/such/home")) else {
        return;
    };
    let snapshots = Snapshots::new(state.path().to_owned(), None);
    assert!(snapshots.refresh(&account, None).is_err());
    assert!(snapshot::load(state.path(), "tester").is_none());
}

#[test]
fn a_user_other_than_the_service_cannot_be_read_without_root() {
    let home = tempfile::tempdir().unwrap();
    let Some(mut account) = own_account(home.path()) else {
        return;
    };
    account.uid += 1;
    assert_eq!(
        drop_to(&account).unwrap_err().kind(),
        io::ErrorKind::PermissionDenied
    );
}
