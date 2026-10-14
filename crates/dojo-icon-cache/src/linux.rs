use std::fmt;
use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

use crate::{Config, Icon, IconCache, IconKey, Lookup, MAX_SOURCE_BYTES, normalize, source_type};

#[derive(Clone, Debug)]
pub struct IconRequest {
    pub app_id: String,
    pub icon_name: String,
    pub scale: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IconAsset {
    Png(Arc<Icon>),
    /// SVG and other formats stay on GPUI's existing file renderer. High-DPI
    /// PNGs also keep their original resolution instead of shrinking to 64px.
    OriginalPath(PathBuf),
}

#[derive(Debug)]
pub enum IconError {
    Io(std::io::Error),
    UntrustedPath,
    NotRegular,
    TooLarge,
    InvalidPng,
    ChangedDuringRead,
    InvalidScale,
    NoTrustedRoots,
}
impl From<std::io::Error> for IconError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}
impl fmt::Display for IconError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "icon read failed: {error}"),
            Self::UntrustedPath => formatter.write_str("icon is outside trusted local roots"),
            Self::NotRegular => formatter.write_str("icon is not a regular local file"),
            Self::TooLarge => formatter.write_str("icon source byte budget exceeded"),
            Self::InvalidPng => formatter.write_str("icon PNG is invalid or exceeds decode limits"),
            Self::ChangedDuringRead => formatter.write_str("icon source changed during read"),
            Self::InvalidScale => formatter.write_str("icon scale must be between one and four"),
            Self::NoTrustedRoots => {
                formatter.write_str("at least one trusted local icon root is required")
            }
        }
    }
}
impl std::error::Error for IconError {}

/// Application-owned cache and Linux resolver. No implicit cache directory is
/// selected. The supplied roots are the host's permitted local icon sources.
pub struct LinuxIcons {
    cache: IconCache,
    roots: Vec<PathBuf>,
}
impl LinuxIcons {
    pub fn new(
        cache_root: Option<PathBuf>,
        config: Config,
        trusted_roots: Vec<PathBuf>,
    ) -> Result<Self, IconError> {
        let roots = trusted_roots
            .into_iter()
            .map(|root| {
                let root = std::fs::canonicalize(root)?;
                if !root.is_dir() {
                    return Err(IconError::NotRegular);
                }
                Ok(root)
            })
            .collect::<Result<Vec<_>, IconError>>()?;
        if roots.is_empty() {
            return Err(IconError::NoTrustedRoots);
        }
        Ok(Self {
            cache: IconCache::new(cache_root, config),
            roots,
        })
    }

    pub fn cache(&self) -> &IconCache {
        &self.cache
    }

    pub async fn get(&self, request: &IconRequest) -> Result<Option<IconAsset>, IconError> {
        if !(1..=4).contains(&request.scale) {
            return Err(IconError::InvalidScale);
        }
        let theme = freedesktop_icons::default_theme_gtk().unwrap_or_else(|| "hicolor".into());
        let missing = key(request, &theme, None);
        if self.cache.lookup(&missing) == Lookup::Negative {
            return Ok(None);
        }
        let resolved = if Path::new(&request.icon_name).is_absolute() {
            Some(PathBuf::from(&request.icon_name))
        } else {
            freedesktop_icons::lookup(&request.icon_name)
                .with_theme(&theme)
                .with_size(24)
                .with_scale(request.scale)
                .with_cache()
                .find()
        };
        let Some(path) = resolved else {
            self.cache.put(&missing, None);
            return Ok(None);
        };
        let canonical = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.cache.put(&missing, None);
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        };
        if !self.roots.iter().any(|root| canonical.starts_with(root)) {
            return Err(IconError::UntrustedPath);
        }
        let metadata = std::fs::metadata(&canonical)?;
        if !metadata.is_file() {
            return Err(IconError::NotRegular);
        }
        if metadata.len() > MAX_SOURCE_BYTES as u64 {
            return Err(IconError::TooLarge);
        }
        let source_key = key(request, &theme, Some((&canonical, &metadata)));
        let mut header = [0; 1024];
        let mut file = self.open_source(&canonical, &metadata)?;
        let head_len = file.read(&mut header)?;
        if source_type(&header[..head_len]) != Some("image/png") || request.scale > 2 {
            // Verify the full bounded local source before returning the same
            // path GPUI already knows how to render. SVG is never a missing PNG.
            let bytes = self.read_source(&canonical, &metadata)?;
            if source_type(&bytes) == Some("image/png") && normalize(&bytes).is_none() {
                return Err(IconError::InvalidPng);
            }
            return Ok(Some(IconAsset::OriginalPath(canonical)));
        }
        self.cache
            .get_or_fetch(&source_key, || async {
                let bytes = self.read_source(&canonical, &metadata)?;
                if normalize(&bytes).is_none() {
                    return Err(IconError::InvalidPng);
                }
                Ok(Some(bytes))
            })
            .await
            .map(|icon| icon.map(IconAsset::Png))
    }

    /// Independent errors stay attached to their item; a corrupt app icon
    /// cannot hide otherwise valid icons in the visible launcher rows.
    pub async fn get_many(
        &self,
        requests: &[IconRequest],
    ) -> Vec<Result<Option<IconAsset>, IconError>> {
        let mut results = Vec::with_capacity(requests.len());
        for request in requests {
            results.push(self.get(request).await);
        }
        results
    }

    fn open_source(&self, path: &Path, expected: &Metadata) -> Result<File, IconError> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        let opened = file.metadata()?;
        if !opened.is_file() {
            return Err(IconError::NotRegular);
        }
        let actual_path = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
        if !self.roots.iter().any(|root| actual_path.starts_with(root)) {
            return Err(IconError::UntrustedPath);
        }
        if stamp(&opened) != stamp(expected) {
            return Err(IconError::ChangedDuringRead);
        }
        Ok(file)
    }
    fn read_source(&self, path: &Path, expected: &Metadata) -> Result<Vec<u8>, IconError> {
        let file = self.open_source(path, expected)?;
        let mut bytes = Vec::new();
        (&file)
            .take(MAX_SOURCE_BYTES as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_SOURCE_BYTES {
            return Err(IconError::TooLarge);
        }
        if stamp(&file.metadata()?) != stamp(expected) {
            return Err(IconError::ChangedDuringRead);
        }
        Ok(bytes)
    }
}

fn stamp(metadata: &Metadata) -> (u64, u64, u64, i64, i64) {
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
    )
}
fn key(request: &IconRequest, theme: &str, source: Option<(&Path, &Metadata)>) -> IconKey {
    let mut version = Sha256::new();
    for part in [request.icon_name.as_bytes(), theme.as_bytes()] {
        version.update((part.len() as u64).to_le_bytes());
        version.update(part);
    }
    version.update(request.scale.to_le_bytes());
    if let Some((path, metadata)) = source {
        version.update(path.as_os_str().as_encoded_bytes());
        version.update(format!("{:?}", stamp(metadata)).as_bytes());
        if let Ok(modified) = metadata.modified().and_then(|time| {
            time.duration_since(UNIX_EPOCH)
                .map_err(std::io::Error::other)
        }) {
            version.update(modified.as_nanos().to_le_bytes());
        }
    }
    IconKey::new(
        "host-linux-icons-v1",
        &request.app_id,
        &hex::encode(version.finalize()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;
    fn png(color: [u8; 4]) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(24, 24, Rgba(color)))
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }
    fn request(path: &Path) -> IconRequest {
        IconRequest {
            app_id: "fixture.desktop".into(),
            icon_name: path.to_string_lossy().into_owned(),
            scale: 1,
        }
    }
    fn icons(root: &Path, disk: Option<PathBuf>) -> LinuxIcons {
        LinuxIcons::new(disk, Config::default(), vec![root.to_owned()]).unwrap()
    }

    #[tokio::test]
    async fn local_png_is_normalized_reused_and_loaded_after_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("app.png");
        std::fs::write(&path, png([255, 0, 0, 255])).unwrap();
        let disk = root.path().join("cache");
        let cache = icons(root.path(), Some(disk.clone()));
        let request = request(&path);
        let IconAsset::Png(first) = cache.get(&request).await.unwrap().unwrap() else {
            panic!("PNG");
        };
        let IconAsset::Png(second) = cache.get(&request).await.unwrap().unwrap() else {
            panic!("PNG");
        };
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.cache.stats().fetch_calls, 1);
        assert_eq!(image::load_from_memory(&first.bytes).unwrap().width(), 64);
        let reopened = icons(root.path(), Some(disk));
        assert_eq!(
            reopened.get(&request).await.unwrap(),
            Some(IconAsset::Png(first))
        );
        assert_eq!(reopened.cache.stats().disk_hits, 1);
    }
    #[tokio::test]
    async fn svg_and_high_density_png_keep_gpui_original_paths() {
        let root = tempfile::tempdir().unwrap();
        let svg = root.path().join("app.svg");
        std::fs::write(
            &svg,
            b"<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24'/>",
        )
        .unwrap();
        let cache = icons(root.path(), None);
        assert_eq!(
            cache.get(&request(&svg)).await.unwrap(),
            Some(IconAsset::OriginalPath(svg))
        );
        let path = root.path().join("app.png");
        std::fs::write(&path, png([0, 255, 0, 255])).unwrap();
        let mut requested = request(&path);
        requested.scale = 4;
        assert_eq!(
            cache.get(&requested).await.unwrap(),
            Some(IconAsset::OriginalPath(path))
        );
        assert_eq!(cache.cache.stats().fetch_calls, 0);
    }
    #[tokio::test]
    async fn untrusted_symlink_and_oversized_file_are_refused_without_negative_cache() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("app.png");
        std::fs::write(&target, png([0, 0, 255, 255])).unwrap();
        let path = root.path().join("app.png");
        std::os::unix::fs::symlink(target, &path).unwrap();
        let cache = icons(root.path(), None);
        assert!(matches!(
            cache.get(&request(&path)).await,
            Err(IconError::UntrustedPath)
        ));
        let large = root.path().join("large.png");
        let file = File::create(&large).unwrap();
        file.set_len(MAX_SOURCE_BYTES as u64 + 1).unwrap();
        assert!(matches!(
            cache.get(&request(&large)).await,
            Err(IconError::TooLarge)
        ));
    }
    #[tokio::test]
    async fn corrupt_icon_does_not_hide_a_valid_batch_item() {
        let root = tempfile::tempdir().unwrap();
        let bad = root.path().join("bad.png");
        std::fs::write(&bad, b"\x89PNG\r\n\x1a\ninvalid").unwrap();
        let good = root.path().join("good.png");
        std::fs::write(&good, png([255, 255, 0, 255])).unwrap();
        let cache = icons(root.path(), None);
        let requests = vec![request(&bad), request(&good)];
        let results = cache.get_many(&requests).await;
        assert!(matches!(&results[0], Err(IconError::InvalidPng)));
        assert!(matches!(&results[1], Ok(Some(IconAsset::Png(_)))));
        std::fs::write(&bad, png([0, 0, 0, 255])).unwrap();
        assert!(matches!(
            cache.get(&requests[0]).await,
            Ok(Some(IconAsset::Png(_)))
        ));
    }
    #[tokio::test]
    async fn absent_icon_is_negative_until_ttl_then_new_file_is_visible() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("new.png");
        let cache = LinuxIcons::new(
            None,
            Config {
                negative_ttl: std::time::Duration::from_millis(1),
                ..Config::default()
            },
            vec![root.path().into()],
        )
        .unwrap();
        assert_eq!(cache.get(&request(&path)).await.unwrap(), None);
        std::fs::write(&path, png([0, 0, 0, 255])).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        assert!(matches!(
            cache.get(&request(&path)).await,
            Ok(Some(IconAsset::Png(_)))
        ));
    }
}

#[cfg(test)]
mod executor_test {
    use super::*;
    use std::future::Future;
    use std::task::{Context, Poll, Wake, Waker};
    struct Noop;
    impl Wake for Noop {
        fn wake(self: Arc<Self>) {}
    }
    #[test]
    fn gpui_executor_does_not_require_a_tokio_runtime() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("icon.png");
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([255, 0, 0, 255]),
        ))
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
        std::fs::write(&path, encoded.into_inner()).unwrap();
        let cache = LinuxIcons::new(None, Config::default(), vec![root.path().into()]).unwrap();
        let request = IconRequest {
            app_id: "app.desktop".into(),
            icon_name: path.to_string_lossy().into_owned(),
            scale: 1,
        };
        let future = cache.get(&request);
        let mut future = std::pin::pin!(future);
        let waker = Waker::from(Arc::new(Noop));
        let mut context = Context::from_waker(&waker);
        assert!(matches!(
            future.as_mut().poll(&mut context),
            Poll::Ready(Ok(Some(IconAsset::Png(_))))
        ));
    }
}
