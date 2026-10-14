use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// A `w` x `h` opaque PNG.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 60, 20, 255]));
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img)
        .write_to(&mut out, image::ImageFormat::Png)
        .unwrap();
    out.into_inner()
}

fn dims(bytes: &[u8]) -> (u32, u32) {
    use image::GenericImageView;
    image::load_from_memory(bytes).unwrap().dimensions()
}

fn cache(dir: &Path, config: Config) -> IconCache {
    IconCache::new(Some(dir.to_path_buf()), config)
}

fn key(app: &str) -> IconKey {
    IconKey::new("guest-linux:sha256:abc", app, "")
}

#[test]
fn keys_are_stable_app_identities() {
    assert_eq!(key("org.xfce.Terminal"), key(" ORG.xfce.terminal "));
    assert_ne!(
        key("firefox"),
        IconKey::new("guest-linux:sha256:def", "firefox", "")
    );
    assert_ne!(
        IconKey::new("host-macos", "com.apple.Safari", "18.0"),
        IconKey::new("host-macos", "com.apple.Safari", "18.1")
    );
    assert_eq!(key("a").stem().len(), 32);
}

#[test]
fn pngs_normalize_to_1x_and_2x_squares() {
    let icon = normalize(&png(256, 128)).unwrap();
    assert_eq!(icon.content_type, "image/png");
    assert_eq!(dims(&icon.bytes), (SIZE_2X, SIZE_2X));
    assert_eq!(dims(&icon.bytes_1x), (SIZE_1X, SIZE_1X));
    assert_eq!(icon.for_size(32), icon.bytes_1x.as_slice());
    assert_eq!(icon.for_size(64), icon.bytes.as_slice());
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";
    let icon = normalize(svg).unwrap();
    assert_eq!(
        (icon.content_type, icon.bytes.as_slice()),
        ("image/svg+xml", &svg[..])
    );
    assert_eq!(normalize(b"/* XPM */"), None);
    assert_eq!(normalize(b""), None);
}

#[tokio::test]
async fn miss_then_memory_hit_then_disk_hit() {
    let dir = tempfile::tempdir().unwrap();
    let c = cache(dir.path(), Config::default());
    assert_eq!(c.lookup(&key("xterm")), Lookup::Miss);
    let got = c
        .get_or_fetch(&key("xterm"), || async { Ok::<_, ()>(Some(png(48, 48))) })
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(c.lookup(&key("xterm")), Lookup::Hit(i) if *i == *got));
    // A new process: memory empty, the disk answers.
    c.clear_memory();
    assert!(matches!(c.lookup(&key("xterm")), Lookup::Hit(i) if *i == *got));
    let s = c.stats();
    assert_eq!((s.fetch_calls, s.memory_hits, s.disk_hits), (1, 1, 1));
}

#[tokio::test]
async fn concurrent_requests_share_one_fetch() {
    let c = Arc::new(IconCache::new(None, Config::default()));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let (c, calls) = (c.clone(), calls.clone());
        tasks.push(tokio::spawn(async move {
            c.get_or_fetch(&key("firefox"), || async {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                Ok::<_, ()>(Some(png(64, 64)))
            })
            .await
            .unwrap()
        }));
    }
    for t in tasks {
        assert!(t.await.unwrap().is_some());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_batch_fetches_every_miss_in_one_call() {
    let c = IconCache::new(None, Config::default());
    c.put(&key("cached"), Some(&png(64, 64)));
    let keys: Vec<IconKey> = ["cached", "a", "b", "c"].iter().map(|k| key(k)).collect();
    let seen = Mutex::new(Vec::new());
    let got = c
        .get_or_fetch_many(&keys, |idx| {
            seen.lock().unwrap().extend(idx.clone());
            async move { Ok::<_, ()>(idx.iter().map(|&i| (i != 2).then(|| png(16, 16))).collect()) }
        })
        .await
        .unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![1, 2, 3]);
    assert_eq!(
        got.iter().map(Option::is_some).collect::<Vec<_>>(),
        [true, true, false, true]
    );
    assert_eq!(c.stats().fetch_calls, 1);
}

#[tokio::test]
async fn negatives_expire_and_failures_are_not_remembered() {
    let c = IconCache::new(
        None,
        Config {
            negative_ttl: Duration::from_millis(80),
            ..Config::default()
        },
    );
    assert!(
        c.get_or_fetch(&key("xcalc"), || async { Ok::<_, ()>(None) })
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(c.lookup(&key("xcalc")), Lookup::Negative);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(c.lookup(&key("xcalc")), Lookup::Miss);
    // With a disk, a new process honors a recent negative too.
    let dir = tempfile::tempdir().unwrap();
    let d = cache(dir.path(), Config::default());
    d.put(&key("xeyes"), None);
    d.clear_memory();
    assert_eq!(d.lookup(&key("xeyes")), Lookup::Negative);
    // A later icon replaces the negative.
    d.put(&key("xeyes"), Some(&png(16, 16)));
    d.clear_memory();
    assert!(matches!(d.lookup(&key("xeyes")), Lookup::Hit(_)));
    // A failed round trip is returned, and the next ask fetches again.
    let err = c
        .get_or_fetch(&key("thunar"), || async {
            Err::<Option<Vec<u8>>, _>("down")
        })
        .await;
    assert_eq!(err, Err("down"));
    assert_eq!(c.lookup(&key("thunar")), Lookup::Miss);
}

#[tokio::test]
async fn memory_and_disk_evict_the_least_recently_used() {
    let dir = tempfile::tempdir().unwrap();
    let one = normalize(&png(64, 64)).unwrap();
    let per_entry = (one.bytes.len() + one.bytes_1x.len()) as u64;
    let c = cache(
        dir.path(),
        Config {
            memory_entries: 2,
            disk_bytes: per_entry * 2,
            ..Config::default()
        },
    );
    for (i, app) in ["a", "b", "c"].iter().enumerate() {
        c.put(&key(app), Some(&png(64, 64)));
        if i == 1 {
            // Touch "a" so "b" is the oldest in memory.
            assert!(matches!(c.lookup(&key("a")), Lookup::Hit(_)));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(c.memory.lock().unwrap().entries.len(), 2);
    let files = std::fs::read_dir(dir.path()).unwrap().count();
    assert!(
        files <= 4,
        "disk kept {files} files over a two-entry budget"
    );
    // The newest entry survived on disk.
    c.clear_memory();
    assert!(matches!(c.lookup(&key("c")), Lookup::Hit(_)));
}

#[test]
fn memory_only_cache_has_no_implicit_directory() {
    let cache = IconCache::new(None, Config::default());
    cache.put(&key("probe"), Some(&png(8, 8)));
    assert_eq!(cache.dir(), None);
}

#[test]
fn previews_are_reused_briefly_and_errors_are_not_kept() {
    let t = Thumbnails::new(Duration::from_millis(80), 2);
    let calls = std::cell::Cell::new(0);
    let capture = || {
        calls.set(calls.get() + 1);
        Ok::<_, ()>(Some(vec![1, 2, 3]))
    };
    assert!(t.get_or_capture("w:1", capture).unwrap().is_some());
    assert!(t.get_or_capture("w:1", capture).unwrap().is_some());
    assert_eq!(calls.get(), 1);
    std::thread::sleep(Duration::from_millis(100));
    assert!(t.get_or_capture("w:1", capture).unwrap().is_some());
    assert_eq!(calls.get(), 2, "a stale preview is captured again");
    assert_eq!(
        t.get_or_capture("w:2", || Err::<Option<Vec<u8>>, _>("denied")),
        Err("denied")
    );
    assert!(t.get("w:2").is_none());
    // Capacity: the oldest goes.
    t.put("a", Some(vec![1]));
    t.put("b", Some(vec![2]));
    t.put("c", Some(vec![3]));
    assert!(t.get("a").is_none() && t.get("c").is_some());
}

#[tokio::test]
async fn canceling_the_fetch_owner_does_not_poison_single_flight() {
    let cache = Arc::new(IconCache::new(None, Config::default()));
    let entered = Arc::new(tokio::sync::Notify::new());
    let owner_cache = cache.clone();
    let notice = entered.clone();
    let owner = tokio::spawn(async move {
        owner_cache
            .get_or_fetch(&key("cancelled"), || async {
                notice.notify_one();
                std::future::pending::<Result<Option<Vec<u8>>, ()>>().await
            })
            .await
    });
    entered.notified().await;
    owner.abort();
    assert!(owner.await.unwrap_err().is_cancelled());
    let fetched = tokio::time::timeout(
        Duration::from_secs(1),
        cache.get_or_fetch(&key("cancelled"), || async {
            Ok::<_, ()>(Some(png(24, 24)))
        }),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(fetched.is_some());
    assert!(cache.inflight.lock().unwrap().is_empty());
}

#[test]
fn replacing_a_positive_with_a_negative_retires_the_old_icon() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(dir.path(), Config::default());
    cache.put(&key("app"), Some(&png(24, 24)));
    cache.put(&key("app"), None);
    assert_eq!(cache.lookup(&key("app")), Lookup::Negative);
    cache.clear_memory();
    assert_eq!(cache.lookup(&key("app")), Lookup::Negative);
}

#[test]
fn changing_cached_source_format_replaces_disk_variant() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(dir.path(), Config::default());
    cache.put(&key("app"), Some(&png(24, 24)));
    cache.put(
        &key("app"),
        Some(b"<svg xmlns='http://www.w3.org/2000/svg'/>"),
    );
    cache.clear_memory();
    let Lookup::Hit(icon) = cache.lookup(&key("app")) else {
        panic!("SVG hit");
    };
    assert_eq!(icon.content_type, "image/svg+xml");
}

#[test]
fn oversized_or_corrupt_disk_entries_are_cache_misses() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(dir.path(), Config::default());
    let path = dir.path().join(format!("{}.png", key("app").stem()));
    let file = std::fs::File::create(&path).unwrap();
    file.set_len(MAX_SOURCE_BYTES as u64 + 1).unwrap();
    assert_eq!(cache.lookup(&key("app")), Lookup::Miss);
    std::fs::write(path, b"\x89PNG\r\n\x1a\ncorrupt").unwrap();
    assert_eq!(cache.lookup(&key("app")), Lookup::Miss);
}

#[test]
fn png_dimension_budget_is_checked_before_allocation() {
    let valid = png(MAX_SOURCE_DIMENSION + 1, 1);
    assert_eq!(
        image::load_from_memory(&valid).unwrap().width(),
        MAX_SOURCE_DIMENSION + 1
    );
    assert!(normalize(&valid).is_none());
}

#[tokio::test]
async fn loading_disk_negative_does_not_restart_its_ttl() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(
        dir.path(),
        Config {
            negative_ttl: Duration::from_millis(250),
            ..Config::default()
        },
    );
    cache.put(&key("missing"), None);
    let path = cache.negative_path(&key("missing")).unwrap();
    std::fs::File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_millis(200))
        .unwrap();
    cache.clear_memory();
    assert_eq!(cache.lookup(&key("missing")), Lookup::Negative);
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(cache.lookup(&key("missing")), Lookup::Miss);
}

#[test]
fn nonregular_disk_entries_refuse_without_blocking() {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(dir.path(), Config::default());
    let path = dir.path().join(format!("{}.png", key("pipe").stem()));
    let name = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert_eq!(cache.lookup(&key("pipe")), Lookup::Miss);
}

#[test]
fn eviction_never_removes_unrelated_files_from_explicit_root() {
    let dir = tempfile::tempdir().unwrap();
    let unrelated = dir.path().join("user-file.png");
    std::fs::write(&unrelated, b"keep me").unwrap();
    let cache = cache(
        dir.path(),
        Config {
            disk_bytes: 1,
            ..Config::default()
        },
    );
    cache.put(&key("app"), Some(&png(24, 24)));
    assert_eq!(std::fs::read(unrelated).unwrap(), b"keep me");
}
