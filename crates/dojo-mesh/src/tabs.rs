//! Which tabs each browser window holds, reported by the "Dojo Tabs" extension (extensions/dojo-tabs).
//!
//! The extension talks to this program through the browser's native messaging: the browser starts
//! `dojo-mesh tabs-host` and writes each report to its standard input as a 4-byte little-endian
//! length and that many bytes of JSON. The host keeps the latest report in a private file under the
//! runtime directory and removes it when the browser lets go, so a file that exists is a live
//! report and a missing one means no list can be trusted.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::{Path, PathBuf};

/// The extension's fixed id (its manifest carries the public key that makes it so). The browser
/// passes the caller's origin to a native host, and only this extension is served.
pub const EXTENSION_ID: &str = "ifbpcpbmclepnkapoahcikkldpjnkgcf";

/// The biggest report accepted, far beyond any real number of tabs.
const MAX_FRAME: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tab {
    pub id: i64,
    pub active: bool,
    #[serde(default)]
    pub pinned: bool,
    pub url: String,
    #[serde(default)]
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserWindow {
    pub id: i64,
    #[serde(default)]
    pub focused: bool,
    pub tabs: Vec<Tab>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub v: u32,
    #[serde(default)]
    pub at: i64,
    pub windows: Vec<BrowserWindow>,
}

impl Snapshot {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let snapshot: Self = serde_json::from_slice(bytes).map_err(|error| format!("not a tab report: {error}"))?;
        if snapshot.v != 1 {
            return Err(format!("tab report version {} is not understood", snapshot.v));
        }
        Ok(snapshot)
    }
}

/// `<runtime>/dojo/browser-tabs.json`.
pub fn snapshot_path(runtime_dir: &Path) -> PathBuf {
    runtime_dir.join("dojo").join("browser-tabs.json")
}

/// The latest live report, or `None` when the extension is not connected (or the file is unreadable).
pub fn load(path: &Path) -> Option<Snapshot> {
    Snapshot::parse(&std::fs::read(path).ok()?).ok()
}

/// The window whose tab in front shows the page a Niri window is titled after. Several windows
/// showing the same page cannot be told apart, so none is chosen then.
pub fn window_for<'a>(snapshot: &'a Snapshot, niri_title: &str, browser_name: &str) -> Option<&'a BrowserWindow> {
    let page_title = niri_title.strip_suffix(&format!(" - {browser_name}")).unwrap_or(niri_title);
    if page_title.is_empty() {
        return None;
    }
    let mut matches = snapshot.windows.iter().filter(|window| window.tabs.iter().any(|tab| tab.active && tab.title == page_title));
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

// ---- the native host ---------------------------------------------------------------------------

/// One native-messaging frame, or `None` at a clean end of input.
pub fn read_frame(input: &mut impl Read) -> Result<Option<Vec<u8>>, String> {
    let mut length = [0u8; 4];
    match input.read(&mut length[..1]) {
        Ok(0) => return Ok(None),
        Ok(_) => {}
        Err(error) => return Err(error.to_string()),
    }
    input.read_exact(&mut length[1..]).map_err(|error| format!("a frame was cut short: {error}"))?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(format!("a frame of {length} bytes is too large"));
    }
    let mut frame = vec![0u8; length];
    input.read_exact(&mut frame).map_err(|error| format!("a frame was cut short: {error}"))?;
    Ok(Some(frame))
}

/// Writes the report where `load` reads it: in a directory only the user can enter, atomically, so
/// a reader never sees half of one.
pub fn store(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    let directory = path.parent().expect("the snapshot has a directory");
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(directory)?;
    let staging = path.with_extension("json.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&staging)?;
    file.write_all(&serde_json::to_vec(snapshot).map_err(std::io::Error::other)?)?;
    file.sync_all().ok();
    std::fs::rename(staging, path)
}

/// Serves one browser connection: keeps `path` current until the browser lets go, then removes it.
/// A malformed frame ends the session, since the extension is the only thing that should be talking.
pub fn serve(input: &mut impl Read, path: &Path) -> Result<(), String> {
    let result = (|| loop {
        match read_frame(input)? {
            None => return Ok(()),
            Some(bytes) => store(path, &Snapshot::parse(&bytes)?).map_err(|error| format!("could not keep the tab report: {error}"))?,
        }
    })();
    let _ = std::fs::remove_file(path);
    result
}

/// `dojo-mesh tabs-host [ORIGIN]`: the browser's native messaging entry point.
pub fn host(origin: Option<&str>, runtime_dir: Option<PathBuf>) -> Result<(), String> {
    let expected = format!("chrome-extension://{EXTENSION_ID}/");
    if origin != Some(expected.as_str()) {
        return Err(format!("this host serves only the Dojo Tabs extension, not {}", origin.unwrap_or("an unknown caller")));
    }
    let runtime_dir = runtime_dir.filter(|dir| dir.is_absolute()).ok_or("there is no runtime directory")?;
    serve(&mut std::io::stdin().lock(), &snapshot_path(&runtime_dir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(json: &str) -> Vec<u8> {
        let mut bytes = (json.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        bytes
    }

    const REPORT: &str = r#"{"v":1,"at":5,"windows":[
      {"id":7,"focused":true,"tabs":[
        {"id":1,"active":false,"pinned":true,"url":"https://a.example/","title":"A"},
        {"id":2,"active":true,"url":"https://b.example/","title":"B page"}]},
      {"id":9,"tabs":[{"id":3,"active":true,"url":"https://c.example/","title":"C"}]}]}"#;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dojo-mesh-tabs-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn the_extension_id_is_the_one_its_manifest_key_makes() {
        use base64::Engine as _;
        use sha2::{Digest, Sha256};
        let manifest: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../extensions/dojo-tabs/manifest.json")).unwrap()).unwrap();
        let key = base64::engine::general_purpose::STANDARD.decode(manifest["key"].as_str().unwrap()).unwrap();
        let id: String = Sha256::digest(&key)[..16].iter().flat_map(|byte| [byte >> 4, byte & 15]).map(|nibble| (b'a' + nibble) as char).collect();
        assert_eq!(id, EXTENSION_ID);
    }

    #[test]
    fn the_home_manager_module_allows_exactly_that_extension() {
        let module = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tabs-home-manager.nix")).unwrap();
        assert!(module.contains(&format!("extensionId = \"{EXTENSION_ID}\";")), "tabs-home-manager.nix names another extension id");
    }

    #[test]
    fn reports_are_read_and_unknown_versions_are_refused() {
        let snapshot = Snapshot::parse(REPORT.as_bytes()).unwrap();
        assert_eq!(snapshot.windows.len(), 2);
        assert_eq!(snapshot.windows[0].tabs[0], Tab { id: 1, active: false, pinned: true, url: "https://a.example/".into(), title: "A".into() });
        assert!(!snapshot.windows[1].focused, "missing fields take their defaults");
        assert!(Snapshot::parse(br#"{"v":2,"windows":[]}"#).is_err());
        assert!(Snapshot::parse(b"{}").is_err());
        assert!(Snapshot::parse(b"nonsense").is_err());
    }

    #[test]
    fn a_niri_window_is_matched_to_the_browser_window_showing_its_page() {
        let snapshot = Snapshot::parse(REPORT.as_bytes()).unwrap();
        assert_eq!(window_for(&snapshot, "B page - Helium", "Helium").unwrap().id, 7);
        assert_eq!(window_for(&snapshot, "C - Helium", "Helium").unwrap().id, 9);
        assert!(window_for(&snapshot, "A - Helium", "Helium").is_none(), "A is a tab behind the front one, not a window's page");
        assert!(window_for(&snapshot, "Unknown - Helium", "Helium").is_none());
        assert!(window_for(&snapshot, " - Helium", "Helium").is_none());
    }

    #[test]
    fn two_windows_showing_the_same_page_are_not_guessed_between() {
        let mut snapshot = Snapshot::parse(REPORT.as_bytes()).unwrap();
        snapshot.windows[1].tabs[0].title = "B page".into();
        assert!(window_for(&snapshot, "B page - Helium", "Helium").is_none());
    }

    #[test]
    fn frames_are_read_whole_and_a_clean_end_is_none() {
        let mut bytes = frame("{\"a\":1}");
        bytes.extend(frame("{\"b\":2}"));
        let mut input = bytes.as_slice();
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{\"a\":1}");
        assert_eq!(read_frame(&mut input).unwrap().unwrap(), b"{\"b\":2}");
        assert_eq!(read_frame(&mut input).unwrap(), None);
        assert!(read_frame(&mut &[5u8, 0, 0, 0, b'x'][..]).is_err(), "a frame cut short");
        assert!(read_frame(&mut &[1u8, 0][..]).is_err(), "a length cut short");
        assert!(read_frame(&mut &(MAX_FRAME as u32 + 1).to_le_bytes()[..]).is_err(), "a frame that is too large");
    }

    #[test]
    fn the_latest_report_is_kept_private_and_removed_when_the_browser_lets_go() {
        use std::os::unix::fs::PermissionsExt;
        let runtime = dir("serve");
        let path = snapshot_path(&runtime);
        let mut bytes = frame(REPORT);
        bytes.extend(frame(r#"{"v":1,"windows":[{"id":1,"tabs":[]}]}"#));
        assert!(path.parent().is_some_and(|parent| !parent.exists()));
        // The file is checked while the session is still running by serving a reader that blocks.
        let (mut reader, mut writer) = std::os::unix::net::UnixStream::pair().unwrap();
        let served = path.clone();
        let handle = std::thread::spawn(move || serve(&mut reader, &served));
        std::io::Write::write_all(&mut writer, &bytes).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while load(&path).is_none_or(|snapshot| snapshot.windows.len() != 1) {
            assert!(std::time::Instant::now() < deadline, "the second report never arrived");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        drop(writer);
        assert!(handle.join().unwrap().is_ok());
        assert!(!path.exists(), "no file means no live report");
        std::fs::remove_dir_all(&runtime).unwrap();
    }

    #[test]
    fn a_malformed_report_ends_the_session_and_leaves_no_file() {
        let runtime = dir("bad");
        let path = snapshot_path(&runtime);
        let mut bytes = frame(REPORT);
        bytes.extend(frame("not json"));
        assert!(serve(&mut bytes.as_slice(), &path).is_err());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&runtime);
    }

    #[test]
    fn only_the_dojo_tabs_extension_is_served() {
        let runtime = Some(std::env::temp_dir());
        assert!(host(Some("chrome-extension://abcdefghijklmnopabcdefghijklmnop/"), runtime.clone()).unwrap_err().contains("serves only the Dojo Tabs extension"));
        assert!(host(None, runtime.clone()).is_err());
        assert!(host(Some(&format!("chrome-extension://{EXTENSION_ID}/")), None).unwrap_err().contains("no runtime directory"));
    }
}
