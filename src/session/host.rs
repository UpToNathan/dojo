//! Host facts the shell and the locker both read: the account, the local hour
//! and the theme files the closed state is drawn from. Best effort throughout.

use std::ffi::CStr;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::RenderImage;

/// Used when the theme's `palette.json` has no readable `error` colour.
const FALLBACK_ERROR: u32 = 0xf7768e;

fn theme_file(name: &str) -> Option<PathBuf> {
    Some(
        crate::theme::Paths::user_default()
            .ok()?
            .root
            .join("current")
            .join(name),
    )
}

/// `colors.error` from the theme's `palette.json`, else the fallback, as `0xrrggbb`.
pub fn error_color() -> u32 {
    theme_file("palette.json")
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| error_colour(&text))
        .unwrap_or(FALLBACK_ERROR)
}

/// `colors.error` from `palette.json`, as `#rrggbb`.
pub fn error_colour(palette: &str) -> Option<u32> {
    let value: serde_json::Value = serde_json::from_str(palette).ok()?;
    let hex = value
        .get("colors")?
        .get("error")?
        .as_str()?
        .strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?
}

/// Decodes the pre-blurred wallpaper the theme writes. Slow enough to run off
/// the UI thread. `None` paints a flat palette colour.
pub fn decode_wallpaper() -> Option<Arc<RenderImage>> {
    let path = theme_file("lock-wallpaper.png")?;
    let image = match image::open(&path) {
        Ok(image) => image,
        Err(error) if path.exists() => {
            eprintln!("dojo: ignoring {}: {error}", path.display());
            return None;
        }
        Err(_) => return None,
    };
    let mut pixels = image.into_rgba8();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(pixels)])))
}

pub struct Account {
    pub login: String,
    pub gecos: Option<String>,
}

impl Account {
    pub fn current() -> Account {
        let entry = unsafe { libc::getpwuid(libc::getuid()) };
        if entry.is_null() {
            return Account {
                login: std::env::var("USER").unwrap_or_default(),
                gecos: None,
            };
        }
        let text = |field: *const libc::c_char| {
            (!field.is_null())
                .then(|| {
                    unsafe { CStr::from_ptr(field) }
                        .to_string_lossy()
                        .into_owned()
                })
                .filter(|text| !text.is_empty())
        };
        let entry = unsafe { &*entry };
        Account {
            login: text(entry.pw_name).unwrap_or_default(),
            gecos: text(entry.pw_gecos),
        }
    }

    /// The full name for the user chip, else the login.
    pub fn full_name(&self) -> String {
        self.gecos
            .as_deref()
            .and_then(|gecos| gecos.split(',').next())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.login)
            .to_owned()
    }
}

pub fn local_hour(unix: i64) -> u8 {
    let mut tm = std::mem::MaybeUninit::<libc::tm>::uninit();
    let timestamp: libc::time_t = unix;
    if unsafe { libc::localtime_r(&timestamp, tm.as_mut_ptr()) }.is_null() {
        return 12;
    }
    unsafe { tm.assume_init() }.tm_hour as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_error_colour_comes_from_the_palette_colours() {
        assert_eq!(
            error_colour(r##"{"colors": {"error": "#db4b4b", "primary": "#7aa2f7"}}"##),
            Some(0xdb4b4b)
        );
        assert_eq!(error_colour(r##"{"colors": {"error": "red"}}"##), None);
        assert_eq!(error_colour(r##"{"error": "#db4b4b"}"##), None);
        assert_eq!(error_colour("not json"), None);
    }

    #[test]
    fn the_user_chip_prefers_the_gecos_full_name() {
        let account = |gecos: Option<&str>| Account {
            login: "alyssa".into(),
            gecos: gecos.map(Into::into),
        };
        assert_eq!(
            account(Some("Alyssa Park,,,")).full_name(),
            "Alyssa Park"
        );
        assert_eq!(account(Some(",,,")).full_name(), "alyssa");
        assert_eq!(account(None).full_name(), "alyssa");
    }
}
