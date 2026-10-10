use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::Path;

use super::{Appearance, SemanticPalette};

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub fn preview(image: &Path, output: &Path) -> anyhow::Result<()> {
    let image = image.canonicalize()?;
    let mut source = std::fs::File::open(&image)?;
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut source)
        .take(50 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() <= 50 * 1024 * 1024, "wallpaper exceeds 50 MiB");
    let palettes = crate::extraction::extract_all_pairs(&bytes)?;
    let uri: String = image
        .to_string_lossy()
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect();
    let mut html = format!(
        "<!doctype html><meta charset=utf-8><title>Dojo theme previews</title><style>body{{font:15px system-ui;background:#202024;color:#eee;margin:32px}}main{{display:grid;grid-template-columns:repeat(auto-fit,minmax(360px,1fr));gap:20px}}article{{border:1px solid #777;border-radius:14px;padding:18px}}.swatches{{display:flex;height:28px}}.swatches span{{flex:1}}.wallpaper{{width:100%;max-height:260px;object-fit:cover}}.sample{{padding:14px;border-radius:10px}}code{{font-size:12px}}.roles{{display:flex;gap:8px;flex-wrap:wrap}}.roles span{{padding:4px 8px;border-radius:4px}}</style><h1>Dojo theme previews</h1><p>{}</p><p>Read-only comparison. Nothing has been applied.</p><img class=wallpaper src=\"file://{uri}\"><main>",
        escape(&image.to_string_lossy())
    );
    for (mode, dark, light) in palettes {
        for (appearance, palette) in [(Appearance::Dark, dark), (Appearance::Light, light)] {
            let semantic = SemanticPalette::derive(&palette, appearance);
            semantic.validate()?;
            html.push_str(&format!("<article style=\"background:{};color:{};border-color:{}\"><h2>{} · {}</h2><div class=swatches>",semantic.color("background"),semantic.color("foreground"),semantic.color("border"),mode.name(),appearance.name()));
            for color in &semantic.terminal {
                html.push_str(&format!(
                    "<span style=\"background:{color}\" title=\"{color}\"></span>"
                ));
            }
            html.push_str(&format!("</div><div class=sample style=\"background:{}\"><strong style=\"color:{}\">Application heading</strong><p>Body text and <span style=\"color:{}\">muted text</span>.</p><p style=\"background:{};color:{};padding:8px\">Selected item</p><div class=roles>",semantic.color("surface"),semantic.color("primary"),semantic.color("foreground_muted"),semantic.color("background_highlight"),semantic.color("selection_foreground")));
            for role in [
                "primary",
                "secondary",
                "error",
                "warning",
                "success",
                "info",
            ] {
                html.push_str(&format!(
                    "<span style=\"color:{}\">{role}</span>",
                    semantic.color(role)
                ));
            }
            html.push_str(&format!(
                "</div></div><p><code>--mode {} --appearance {}</code></p></article>",
                mode.name(),
                appearance.name()
            ));
        }
    }
    html.push_str("</main>");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    file.write_all(html.as_bytes())?;
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_escapes_markup() {
        assert_eq!(escape("<a>&\""), "&lt;a&gt;&amp;&quot;");
    }
}
