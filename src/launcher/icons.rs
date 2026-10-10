use std::path::{Path, PathBuf};

pub fn resolve(name: &str, scale: u16) -> Option<PathBuf> {
    if Path::new(name).is_absolute() {
        return Path::new(name).is_file().then(|| PathBuf::from(name));
    }
    let theme = freedesktop_icons::default_theme_gtk().unwrap_or_else(|| "hicolor".into());
    freedesktop_icons::lookup(name)
        .with_theme(&theme)
        .with_size(24)
        .with_scale(scale)
        .with_cache()
        .find()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_icons_are_supported_without_theme_lookup() -> anyhow::Result<()> {
        let path = std::env::temp_dir().join(format!("dojo-icon-{}.png", std::process::id()));
        std::fs::write(&path, b"fixture")?;
        assert_eq!(resolve(&path.to_string_lossy(), 1), Some(path.clone()));
        std::fs::remove_file(&path)?;
        assert_eq!(resolve(&path.to_string_lossy(), 1), None);
        Ok(())
    }
}
