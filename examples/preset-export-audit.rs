use dojo::theme::{Theme, presets};

fn main() -> anyhow::Result<()> {
    let root =
        std::path::PathBuf::from(std::env::args().nth(1).expect("output directory required"));
    for preset in presets()? {
        let directory = root.join(&preset.id);
        std::fs::create_dir_all(&directory)?;
        let mut theme = Theme::from_palette(preset.palette)?;
        theme.preset = Some(preset.id.clone());
        for (name, bytes) in theme.outputs(preset.appearance)? {
            std::fs::write(directory.join(name), bytes)?;
        }
    }
    Ok(())
}
