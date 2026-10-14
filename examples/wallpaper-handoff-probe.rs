//! Scoped live wallpaper probe: keeps the palette and restores the exact activation.
use anyhow::Context as _;
use dojo::theme::{Paths, Selection, ThemeStore};
use std::{os::unix::fs::symlink, path::PathBuf, time::Duration};
struct Restore {
    root: PathBuf,
    original: PathBuf,
}
impl Drop for Restore {
    fn drop(&mut self) {
        let next = self
            .root
            .join(format!(".probe-restore-{}", std::process::id()));
        if let Err(error) = (|| -> std::io::Result<()> {
            symlink(&self.original, &next)?;
            std::fs::rename(next, self.root.join("current"))
        })() {
            eprintln!("Wallpaper probe restore failed: {error}");
        }
    }
}
fn main() -> anyhow::Result<()> {
    let paths = std::env::args()
        .skip(1)
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    anyhow::ensure!(paths.len() == 2, "requires two fixture images");
    let store = ThemeStore::new(Paths::user_default()?);
    let _lock = store.lock()?;
    let original = std::fs::read_link(store.paths.root.join("current"))?;
    let _restore = Restore {
        root: store.paths.root.clone(),
        original,
    };
    let selected = store.selection()?.context("select a theme first")?;
    let mut selections = Vec::new();
    for path in paths {
        let theme = store
            .theme(&selected.generation)?
            .with_wallpaper_snapshot(&path, std::fs::read(&path)?)?;
        selections.push(Selection {
            generation: store.generate(&theme)?,
            ..selected.clone()
        });
    }
    for (index, label) in [(0, "CALIBRATE_A"), (1, "CALIBRATE_B")] {
        store.commit(&selections[index])?;
        std::thread::sleep(Duration::from_millis(600));
        println!("{label}");
        std::thread::sleep(Duration::from_millis(200));
    }
    store.commit(&selections[0])?;
    std::thread::sleep(Duration::from_millis(400));
    println!("READY");
    for i in 0..12 {
        store.commit(&selections[(i + 1) % 2])?;
        std::thread::sleep(Duration::from_millis(220));
    }
    println!("DONE");
    std::thread::sleep(Duration::from_millis(50));
    Ok(())
}
