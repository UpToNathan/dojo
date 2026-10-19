//! The looks the login screen can paint: one per user whose snapshot has been read, and the
//! default theme for everyone else. Snapshots come from the display manager's state directory
//! and nowhere else; a home directory is never opened.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use dojo::dm::greeter::{self, Answer};
use dojo::session::host::wallpaper_from_png;
use dojo::session::paint::Look;
use dojo::session::snapshot;
use gpui::RenderImage;

/// Where `dojo-dm` keeps its look snapshots. Test builds may point it elsewhere with `DOJO_DM_STATE`.
#[cfg(not(feature = "fake-pam-stack"))]
pub fn state_directory() -> PathBuf {
    PathBuf::from("/var/lib/dojo-dm")
}

#[cfg(feature = "fake-pam-stack")]
pub fn state_directory() -> PathBuf {
    std::env::var_os("DOJO_DM_STATE").map_or_else(|| PathBuf::from("/var/lib/dojo-dm"), PathBuf::from)
}

/// A user's snapshot, read and decoded off the UI thread. `Answer` is carried along so the
/// reply that asked for it can be finished when it lands.
pub struct Loaded {
    pub answer: Answer,
    pub snapshot: Option<snapshot::Look>,
    pub wallpaper: Option<Arc<RenderImage>>,
}

/// Reads `answer`'s snapshot from `root` and decodes its wallpaper. A missing, invalid or
/// partly invalid snapshot is `None`, which paints the default theme.
pub fn load(root: &Path, answer: Answer) -> Loaded {
    let snapshot = snapshot::load(root, &answer.login);
    let wallpaper = snapshot
        .as_ref()
        .and_then(|look| look.wallpaper.as_ref())
        .and_then(|wallpaper| wallpaper_from_png(wallpaper.as_bytes()));
    Loaded {
        answer,
        snapshot,
        wallpaper,
    }
}

struct Entry {
    snapshot: Option<snapshot::Look>,
    look: Rc<Look>,
}

pub struct Looks {
    entries: BTreeMap<String, Entry>,
    default: Rc<Look>,
    /// The user whose look stands in when none is asked for.
    resting: String,
}

fn paint(snapshot: Option<&snapshot::Look>, wallpaper: Option<Arc<RenderImage>>) -> Rc<Look> {
    let (colors, error) = greeter::palette(snapshot);
    Rc::new(Look::new(colors, gpui::rgb(error), wallpaper))
}

impl Looks {
    pub fn new(resting: &str) -> Looks {
        Looks {
            entries: BTreeMap::new(),
            default: paint(None, None),
            resting: resting.to_owned(),
        }
    }

    pub fn insert(&mut self, loaded: &Loaded) {
        let look = paint(loaded.snapshot.as_ref(), loaded.wallpaper.clone());
        self.entries.insert(
            loaded.answer.login.clone(),
            Entry {
                snapshot: loaded.snapshot.clone(),
                look,
            },
        );
    }

    /// The snapshot already read for `login`: `Some(None)` is a user known to have none.
    pub fn read(&self, login: &str) -> Option<Option<&snapshot::Look>> {
        self.entries.get(login).map(|entry| entry.snapshot.as_ref())
    }

    /// The look for `login`, or the resting user's for `None`. A user not read yet gets the default.
    pub fn look(&self, login: Option<&str>) -> Rc<Look> {
        let login = login.unwrap_or(&self.resting);
        self.entries
            .get(login)
            .map_or_else(|| self.default.clone(), |entry| entry.look.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dojo::session::host::FALLBACK_ERROR;
    use dojo::session::settings::SessionSettings;
    use dojo::session::snapshot::{Line, Login, Palette, Wallpaper};
    use gpui::Hsla;

    fn answer(login: &str) -> Answer {
        Answer {
            login: login.into(),
            display_name: None,
            fingerprint: false,
        }
    }

    fn png() -> Vec<u8> {
        let mut bytes = Vec::new();
        image::RgbaImage::new(48, 27)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn stored(root: &Path, name: &str) -> snapshot::Look {
        let preset = &dojo::theme::presets().unwrap()[0];
        let mut look = snapshot::Look::empty(Login::parse(name).unwrap());
        look.full_name = Line::parse("Alyssa Park");
        look.palette = Some(Palette::parse(serde_json::to_vec(&preset.palette).unwrap()).unwrap());
        look.wallpaper = Some(Wallpaper::parse(png()).unwrap());
        look.settings = Some(SessionSettings::default());
        snapshot::store(root, &look).unwrap();
        look
    }

    #[test]
    fn a_stored_snapshot_loads_with_its_wallpaper_decoded() {
        let root = tempfile::tempdir().unwrap();
        let expected = stored(root.path(), "alyssa");
        let loaded = load(root.path(), answer("alyssa"));
        assert_eq!(loaded.snapshot, Some(expected));
        let image = loaded.wallpaper.expect("the wallpaper decodes");
        assert_eq!(image.size(0).width.0, 48);
    }

    #[test]
    fn a_user_without_a_snapshot_or_with_an_unsafe_name_loads_nothing() {
        let root = tempfile::tempdir().unwrap();
        stored(root.path(), "alyssa");
        for name in ["ghost", "Bob", "../alyssa", "", "alyssa/../alyssa"] {
            let loaded = load(root.path(), answer(name));
            assert_eq!(loaded.snapshot, None, "{name:?}");
            assert!(loaded.wallpaper.is_none(), "{name:?}");
        }
    }

    #[test]
    fn a_missing_state_directory_is_the_default_theme_not_an_error() {
        let loaded = load(Path::new("/nonexistent/dojo-dm"), answer("alyssa"));
        assert!(loaded.snapshot.is_none());
    }

    fn background(look: &Look) -> Hsla {
        look.palette.background.into()
    }

    #[test]
    fn each_user_paints_their_own_look_and_strangers_the_default() {
        let root = tempfile::tempdir().unwrap();
        stored(root.path(), "alyssa");
        let mut looks = Looks::new("alyssa");
        let default = background(&looks.look(Some("alyssa")));
        assert_eq!(default, background(&looks.look(Some("anyone"))));
        looks.insert(&load(root.path(), answer("alyssa")));
        looks.insert(&load(root.path(), answer("ghost")));
        assert_ne!(background(&looks.look(Some("alyssa"))), default);
        assert_eq!(background(&looks.look(Some("ghost"))), default);
        // The resting look is the first user's, and what is known about each is remembered.
        assert_eq!(background(&looks.look(None)), background(&looks.look(Some("alyssa"))));
        assert!(matches!(looks.read("alyssa"), Some(Some(_))));
        assert!(matches!(looks.read("ghost"), Some(None)));
        assert!(looks.read("bob").is_none());
    }

    #[test]
    fn the_default_theme_has_the_lock_screens_error_colour() {
        let looks = Looks::new("nobody");
        let error: Hsla = looks.look(None).palette.error.into();
        assert_eq!(error, gpui::rgb(FALLBACK_ERROR).into());
    }
}
