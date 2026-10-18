//! Monitor-sized wallpapers, kept so a start does not decode and resize the source again.
//!
//! Decoding the source and filtering it down to a display costs about 1.7 s of CPU on this
//! machine, and the wallpaper is blank until the decode ends. A prepared image is the final
//! pixels in the order [`gpui::RenderImage`] holds them, in a file named for the source's
//! content digest and the size, so a new wallpaper never reads a stale file. Files are written
//! to a temporary name and renamed, so a reader only ever sees whole files.

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Wallpapers kept at once; older ones are removed when a new one is stored.
const KEPT_DIGESTS: usize = 4;
const EXTENSION: &str = "bgra";

pub struct Prepared {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .map(|home| home.join(".cache"))
        })?;
    Some(base.join("dojo/wallpaper"))
}

fn hex(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn file_name(digest: &[u8; 32], width: u32, height: u32) -> String {
    format!("{}-{width}x{height}.{EXTENSION}", hex(digest))
}

/// `("<digest>", width, height)` for a name this module wrote.
fn parse_name(name: &str) -> Option<(&str, u32, u32)> {
    let stem = name.strip_suffix(&format!(".{EXTENSION}"))?;
    let (digest, size) = stem.split_once('-')?;
    let (width, height) = size.split_once('x')?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(())?;
    Some((digest, width.parse().ok()?, height.parse().ok()?))
}

/// Every prepared size stored for `digest`. A file whose length does not match its name is
/// ignored, so a damaged cache costs one decode and nothing else.
pub fn load(directory: &Path, digest: &[u8; 32]) -> Vec<Prepared> {
    let wanted = hex(digest);
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let (found, width, height) = parse_name(name.to_str()?)?;
            if found != wanted {
                return None;
            }
            let pixels = fs::read(entry.path()).ok()?;
            (width > 0
                && height > 0
                && pixels.len() as u64 == u64::from(width) * u64::from(height) * 4)
                .then_some(Prepared { width, height, pixels })
        })
        .collect()
}

pub fn store(
    directory: &Path,
    digest: &[u8; 32],
    width: u32,
    height: u32,
    pixels: &[u8],
) -> std::io::Result<()> {
    if width == 0 || height == 0 || pixels.len() as u64 != u64::from(width) * u64::from(height) * 4
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "prepared wallpaper pixels do not match their size",
        ));
    }
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let name = file_name(digest, width, height);
    let temporary = directory.join(format!(".{name}.{}", std::process::id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(pixels)?;
        file.sync_data()?;
        fs::rename(&temporary, directory.join(&name))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return result;
    }
    prune(directory, digest);
    Ok(())
}

/// Keeps the files of `current` and of the most recently written other wallpapers, and removes
/// leftover temporary files from a write that was interrupted.
fn prune(directory: &Path, current: &[u8; 32]) {
    let current = hex(current);
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut others: Vec<(String, SystemTime)> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.starts_with('.') {
            let _ = fs::remove_file(entry.path());
            continue;
        }
        let Some((digest, ..)) = parse_name(name) else { continue };
        if digest == current {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match others.iter_mut().find(|(known, _)| known == digest) {
            Some((_, newest)) => *newest = (*newest).max(modified),
            None => others.push((digest.to_owned(), modified)),
        }
    }
    others.sort_by(|a, b| b.1.cmp(&a.1));
    let stale: Vec<_> = others
        .into_iter()
        .skip(KEPT_DIGESTS - 1)
        .map(|(digest, _)| digest)
        .collect();
    if stale.is_empty() {
        return;
    }
    for entry in fs::read_dir(directory).into_iter().flatten().flatten() {
        let name = entry.file_name();
        if let Some((digest, ..)) = name.to_str().and_then(parse_name)
            && stale.iter().any(|stale| stale == digest)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// The prepared size to show before a window has reported its own: among the sizes whose shape
/// is as close to the display's as any (rounding to the pixel makes 2881x1801 and 1440x900 the
/// same shape), the largest, so the first frame is never softer than it must be. The window
/// prepares the exact size afterwards if this is not it.
pub fn best_for<T>(
    items: &[T],
    size: impl Fn(&T) -> (u32, u32),
    width: f32,
    height: f32,
) -> Option<&T> {
    const SAME_SHAPE: f32 = 0.01;
    let area = |item: &T| {
        let (w, h) = size(item);
        u64::from(w) * u64::from(h)
    };
    if width <= 0.0 || height <= 0.0 {
        return items.iter().max_by_key(|item| area(item));
    }
    let wanted = width / height;
    let distance = |item: &T| {
        let (w, h) = size(item);
        (w as f32 / h as f32 - wanted).abs()
    };
    let closest = items.iter().map(distance).min_by(f32::total_cmp)?;
    items
        .iter()
        .filter(|item| distance(item) <= closest + SAME_SHAPE)
        .max_by_key(|item| area(item))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn pixels(width: u32, height: u32, fill: u8) -> Vec<u8> {
        vec![fill; (width * height * 4) as usize]
    }

    #[test]
    fn a_stored_wallpaper_loads_back_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        store(directory.path(), &digest(1), 4, 3, &pixels(4, 3, 9)).unwrap();
        store(directory.path(), &digest(1), 2, 2, &pixels(2, 2, 7)).unwrap();
        let mut loaded = load(directory.path(), &digest(1));
        loaded.sort_by_key(|p| p.width);
        assert_eq!(loaded.len(), 2);
        assert_eq!((loaded[0].width, loaded[0].height), (2, 2));
        assert_eq!(loaded[1].pixels, pixels(4, 3, 9));
    }

    #[test]
    fn another_wallpaper_never_loads_a_stale_file() {
        let directory = tempfile::tempdir().unwrap();
        store(directory.path(), &digest(1), 4, 3, &pixels(4, 3, 9)).unwrap();
        assert!(load(directory.path(), &digest(2)).is_empty());
        assert!(load(&directory.path().join("missing"), &digest(1)).is_empty());
    }

    #[test]
    fn a_damaged_file_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        store(directory.path(), &digest(1), 4, 3, &pixels(4, 3, 9)).unwrap();
        let path = directory.path().join(file_name(&digest(1), 4, 3));
        fs::write(&path, [0u8; 10]).unwrap();
        assert!(load(directory.path(), &digest(1)).is_empty());
    }

    #[test]
    fn pixels_that_do_not_match_their_size_are_refused() {
        let directory = tempfile::tempdir().unwrap();
        assert!(store(directory.path(), &digest(1), 4, 3, &[0; 5]).is_err());
        assert!(store(directory.path(), &digest(1), 0, 3, &[]).is_err());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn only_the_newest_wallpapers_are_kept_and_temporary_files_go() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join(".interrupted"), b"x").unwrap();
        for seed in 1..=6 {
            store(directory.path(), &digest(seed), 2, 2, &pixels(2, 2, seed)).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!directory.path().join(".interrupted").exists());
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), KEPT_DIGESTS);
        assert!(!load(directory.path(), &digest(6)).is_empty());
        assert!(!load(directory.path(), &digest(5)).is_empty());
        assert!(load(directory.path(), &digest(1)).is_empty());
    }

    #[test]
    fn the_same_shape_prefers_the_largest_and_a_different_shape_loses() {
        let sizes = [(1920, 1080), (2881, 1801), (1440, 900)];
        let size = |item: &(u32, u32)| *item;
        assert_eq!(best_for(&sizes, size, 1440.0, 900.0), Some(&(2881, 1801)));
        assert_eq!(best_for(&sizes, size, 3440.0, 1440.0), Some(&(1920, 1080)));
        assert_eq!(best_for(&[], size, 1.0, 1.0), None);
        assert_eq!(best_for(&sizes, size, 0.0, 0.0), Some(&(2881, 1801)));
    }

    #[test]
    fn names_round_trip_and_foreign_files_are_not_parsed() {
        let name = file_name(&digest(0xab), 2880, 1800);
        assert_eq!(parse_name(&name), Some((hex(&digest(0xab)).as_str(), 2880, 1800)));
        assert_eq!(parse_name("notes.txt"), None);
        assert_eq!(parse_name("zz-1x1.bgra"), None);
    }
}
