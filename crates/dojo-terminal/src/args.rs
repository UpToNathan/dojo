//! The command line, parsed by hand.

#[derive(Debug, Clone, PartialEq)]
pub struct Panel {
    /// The output's connector name, such as `eDP-1`. Without one the default output is used.
    pub output: Option<String>,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Args {
    /// `--session LABEL`, as given.
    pub session: Option<String>,
    /// The session to open, settled at start; None for a bare `--picker`.
    pub target: Option<dojo_rex::Target>,
    /// Open the most recently used session, or focus the window already showing it.
    pub reattach: bool,
    /// Open on the session picker.
    pub picker: bool,
    pub cwd: Option<String>,
    pub panel: Option<Panel>,
    pub demo: bool,
    pub command: Vec<String>,
}

pub const USAGE: &str = "usage: dojo-terminal [--session LABEL | --reattach] [--picker] [--cwd DIR] [--demo] \
[--panel [--output NAME] [--x X] [--y Y] [--width W] [--height H]] [-- COMMAND ARGS...]";

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = args.into_iter();
    let mut parsed = Args { session: None, target: None, reattach: false, picker: false, cwd: None, panel: None, demo: false, command: Vec::new() };
    let mut panel = false;
    let mut output = None;
    let (mut x, mut y, mut width, mut height) = (0.0, 0.0, 800.0, 400.0);
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "--session" => parsed.session = Some(value("--session")?),
            "--reattach" => parsed.reattach = true,
            "--picker" => parsed.picker = true,
            "--cwd" => parsed.cwd = Some(value("--cwd")?),
            "--demo" => parsed.demo = true,
            "--panel" => panel = true,
            "--output" => output = Some(value("--output")?),
            "--x" => x = number("--x", value("--x")?)?,
            "--y" => y = number("--y", value("--y")?)?,
            "--width" => width = number("--width", value("--width")?)?,
            "--height" => height = number("--height", value("--height")?)?,
            "--" => {
                parsed.command = args.by_ref().collect();
                break;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    if panel {
        parsed.panel = Some(Panel { output, x, y, width, height });
    } else if output.is_some() {
        return Err("--output needs --panel".into());
    }
    if parsed.reattach && parsed.session.is_some() {
        return Err("--reattach and --session are alternatives".into());
    }
    Ok(parsed)
}

fn number(name: &str, text: String) -> Result<f32, String> {
    match text.parse::<f32>() {
        Ok(value) if value.is_finite() && value >= 0.0 => Ok(value),
        _ => Err(format!("{name} needs a non-negative number, not {text}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_all(args: &[&str]) -> Result<Args, String> {
        parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn defaults_to_a_window_on_the_dojo_session() {
        let args = parse_all(&[]).unwrap();
        assert_eq!(args.session, None);
        assert_eq!(args.panel, None);
        assert!(args.command.is_empty());
    }

    #[test]
    fn a_panel_takes_its_output_and_rect() {
        let args = parse_all(&[
            "--session", "dojo-quick", "--panel", "--output", "eDP-1", "--x", "10", "--y", "20", "--width", "900",
            "--height", "300", "--", "rush", "-l",
        ])
        .unwrap();
        assert_eq!(args.session.as_deref(), Some("dojo-quick"));
        assert_eq!(
            args.panel,
            Some(Panel { output: Some("eDP-1".into()), x: 10.0, y: 20.0, width: 900.0, height: 300.0 })
        );
        assert_eq!(args.command, ["rush", "-l"]);
    }

    #[test]
    fn reattach_and_picker_are_flags() {
        let args = parse_all(&["--reattach", "--picker"]).unwrap();
        assert!(args.reattach && args.picker);
        assert!(parse_all(&["--reattach", "--session", "a"]).is_err());
    }

    #[test]
    fn bad_input_is_refused() {
        assert!(parse_all(&["--bogus"]).is_err());
        assert!(parse_all(&["--session"]).is_err());
        assert!(parse_all(&["--panel", "--width", "wide"]).is_err());
        assert!(parse_all(&["--output", "eDP-1"]).is_err());
    }
}
