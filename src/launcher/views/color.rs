//! The Pick Color command: pick a colour from the screen, then copy it in the format you want.
//!
//! The launcher cannot take the click itself, so choosing "Pick" closes it, Niri's picker runs, and
//! the launcher reopens on this view showing the colour that was picked.

use std::path::{Path, PathBuf};
use std::time::Duration;

use dojo::launcher_core::color::{self, Color, ColorAnswer};
use dojo::launcher_core::commands::CommandId;
use dojo::launcher_core::rows::{Row, RowAction, RowIcon};

use super::{Host, Notify, Outcome, View};

/// Time for the launcher's closing animation to finish before the picker takes the screen.
const CLOSE_DELAY: Duration = Duration::from_millis(300);

type Picker = fn() -> anyhow::Result<Option<Color>>;

pub struct ColorView {
    state_path: Option<PathBuf>,
    color: Option<Color>,
    picker: Picker,
}

/// Where the last picked colour is kept between the picker and the reopened launcher.
fn state_path() -> Option<PathBuf> {
    let state_home = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(std::env::var_os("HOME")?).join(".local/state"),
    };
    state_home
        .is_absolute()
        .then(|| state_home.join("dojo/picked-color.txt"))
}

fn save(path: &Path, color: Color) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension(format!("txt.{}.tmp", std::process::id()));
    std::fs::write(&temporary, color.to_hex())?;
    std::fs::rename(temporary, path)
}

/// The colour saved by the last pick, if the file is there and holds one.
fn load(path: &Path) -> Option<Color> {
    color::parse(std::fs::read_to_string(path).ok()?.trim())
}

/// Asks Niri to pick a colour from the screen. `None` means the pick was cancelled.
fn pick_with_niri() -> anyhow::Result<Option<Color>> {
    let mut socket = niri_ipc::socket::Socket::connect()?;
    match socket
        .send(niri_ipc::Request::PickColor)?
        .map_err(anyhow::Error::msg)?
    {
        niri_ipc::Response::PickedColor(picked) => Ok(picked.map(|picked| {
            color::from_unit_rgb(
                picked.rgb[0] as f32,
                picked.rgb[1] as f32,
                picked.rgb[2] as f32,
            )
        })),
        _ => anyhow::bail!("Niri answered the color picker with something unexpected"),
    }
}

/// The job that runs after the launcher closes: pick, remember, and reopen on the result.
fn pick_and_reopen(picker: Picker, state_path: Option<PathBuf>) {
    std::thread::sleep(CLOSE_DELAY);
    match picker() {
        Ok(Some(color)) => {
            if let Some(path) = &state_path {
                if let Err(error) = save(path, color) {
                    eprintln!("Dojo could not remember the picked color: {error}");
                }
            }
            let command = dojo::control::ControlCommand::LauncherView(CommandId::PickColor);
            if let Err(error) = dojo::control::send(&command) {
                eprintln!("Dojo could not reopen the launcher: {error:#}");
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("Dojo could not pick a color: {error:#}"),
    }
}

impl ColorView {
    pub fn new() -> Self {
        Self {
            state_path: state_path(),
            color: None,
            picker: pick_with_niri,
        }
    }

    fn format_rows(answer: &ColorAnswer) -> Vec<Row> {
        let swatch = [answer.color.r, answer.color.g, answer.color.b];
        let mut rows = answer
            .formats
            .iter()
            .map(|format| Row {
                id: String::new(),
                title: format.text.clone(),
                subtitle: format.label.to_owned(),
                icon: RowIcon::Swatch(swatch),
                action: RowAction::Copy(format.text.clone()),
            })
            .collect::<Vec<_>>();
        rows.push(Row {
            id: String::new(),
            title: format!(
                "Contrast {} on white · {} on black",
                answer.on_white.text(),
                answer.on_black.text()
            ),
            subtitle: "WCAG contrast".into(),
            icon: RowIcon::Swatch(swatch),
            action: RowAction::Copy(answer.on_white.ratio_text()),
        });
        rows
    }
}

impl View for ColorView {
    fn placeholder(&self) -> &str {
        "Type a color or pick one…"
    }

    fn open(&mut self, _notify: Notify) {
        self.color = self.state_path.as_deref().and_then(load);
    }

    fn rows(&mut self, query: &str) -> Vec<Row> {
        let query = query.trim();
        let mut rows = Vec::new();
        // A typed colour (any form, including a bare name) shows its formats in place of the
        // picker: this view is where colours are asked for.
        let typed = (!query.is_empty())
            .then(|| color::evaluate(query))
            .flatten();
        if let Some(answer) = typed {
            return Self::format_rows(&answer);
        }
        if query.is_empty() || "pick a color from the screen".contains(&query.to_lowercase()) {
            rows.push(Row {
                id: String::new(),
                title: "Pick a color from the screen".into(),
                subtitle: "Click anywhere; Esc cancels".into(),
                icon: RowIcon::Glyph("🎨".into()),
                action: RowAction::Item(0),
            });
        }
        if query.is_empty() {
            if let Some(picked) = self.color {
                rows.push(Row::header("Last picked"));
                rows.extend(Self::format_rows(&color::answer(picked)));
            }
        }
        rows
    }

    fn activate(&mut self, row: &Row, _query: &str, _host: &Host) -> Outcome {
        match row.action {
            RowAction::Item(0) => {
                let (picker, path) = (self.picker, self.state_path.clone());
                Outcome::After(Box::new(move || pick_and_reopen(picker, path)))
            }
            _ => Outcome::Stay,
        }
    }

    fn empty_text(&self, _query: &str) -> String {
        "Not a color".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(directory: &Path) -> ColorView {
        ColorView {
            state_path: Some(directory.join("dojo/picked-color.txt")),
            color: None,
            picker: || {
                Ok(Some(Color {
                    r: 255,
                    g: 136,
                    b: 0,
                    a: 255,
                }))
            },
        }
    }

    #[test]
    fn the_empty_view_offers_the_picker_first_and_the_last_picked_colour_after() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path());
        view.open(std::sync::Arc::new(|| {}));
        let rows = view.rows("");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "Pick a color from the screen");

        save(
            view.state_path.as_deref().unwrap(),
            Color {
                r: 255,
                g: 136,
                b: 0,
                a: 255,
            },
        )
        .unwrap();
        view.open(std::sync::Arc::new(|| {}));
        let rows = view.rows("");
        assert_eq!(rows[1].title, "Last picked");
        assert_eq!(rows[2].title, "#ff8800");
        assert_eq!(rows[2].icon, RowIcon::Swatch([255, 136, 0]));
        assert_eq!(rows[2].action, RowAction::Copy("#ff8800".into()));
        assert!(rows.last().unwrap().title.starts_with("Contrast "));
        assert!(rows.iter().any(|row| row.title.starts_with("rgb(")));
    }

    #[test]
    fn a_typed_colour_shows_its_formats_and_anything_else_shows_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path());
        let rows = view.rows("rgb(255, 136, 0)");
        assert_eq!(rows[0].action, RowAction::Copy("#ff8800".into()));
        // A bare name is a colour here, unlike in the root list.
        assert!(view.rows("orange").iter().any(|row| row.title == "#ffa500"));
        assert!(view.rows("not a colour at all").is_empty());
        assert_eq!(view.empty_text("x"), "Not a color");
        // Typing part of the picker's title keeps it findable.
        assert_eq!(view.rows("pick")[0].title, "Pick a color from the screen");
    }

    #[test]
    fn choosing_the_picker_closes_the_launcher_and_runs_a_job() {
        let directory = tempfile::tempdir().unwrap();
        let mut view = view(directory.path());
        let rows = view.rows("");
        let outcome = view.activate(&rows[0], "", &Host::default());
        assert!(matches!(outcome, Outcome::After(_)));
        assert_eq!(
            view.activate(&Row::header("x"), "", &Host::default()),
            Outcome::Stay
        );
    }

    #[test]
    fn the_saved_colour_round_trips_and_a_missing_or_broken_file_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("dojo/picked-color.txt");
        assert_eq!(load(&path), None);
        save(
            &path,
            Color {
                r: 1,
                g: 2,
                b: 3,
                a: 255,
            },
        )
        .unwrap();
        assert_eq!(
            load(&path),
            Some(Color {
                r: 1,
                g: 2,
                b: 3,
                a: 255
            })
        );
        std::fs::write(&path, "not a colour").unwrap();
        assert_eq!(load(&path), None);
    }
}
