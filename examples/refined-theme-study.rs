use anyhow::Context as _;
use dojo::theme::derive_wallpaper_modes;
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let output = args
        .next()
        .context("usage: refined-theme-study OUTPUT.json WALLPAPER...")?;
    let mut studies = Vec::new();
    for path in args {
        let themes = derive_wallpaper_modes(&std::fs::read(&path)?)?;
        studies.push(serde_json::json!({"wallpaper": path, "themes": themes}));
    }
    anyhow::ensure!(!studies.is_empty(), "at least one wallpaper is required");
    std::fs::write(output, serde_json::to_vec_pretty(&studies)?)?;
    Ok(())
}
