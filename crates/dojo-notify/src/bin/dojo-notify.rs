//! `dojo-notify`: send a notice, or change the volume or brightness and say what it is now.
//!
//! ```text
//! dojo-notify send [--kind K] [--urgency low|normal|critical] [--icon I] [--body B]
//!                  [--value 0-100] [--timeout MS] SUMMARY
//! dojo-notify volume up|down|mute [--step N]
//! dojo-notify brightness up|down [--device D] [--step N]
//! ```
//!
//! A level that was changed is the command's success; a notice that could not be sent is only
//! reported on stderr.

use std::process::{Command, ExitCode};

use dojo_notify::level::{self, Volume};
use dojo_notify::{Notice, Urgency, send};

const SINK: &str = "@DEFAULT_AUDIO_SINK@";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("send") => send_command(&args[1..]),
        Some("volume") => volume(&args[1..]),
        Some("brightness") => brightness(&args[1..]),
        _ => Err("usage: dojo-notify send|volume|brightness ...".to_owned()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("dojo-notify: {message}");
            ExitCode::FAILURE
        }
    }
}

fn announce(notice: Notice) {
    if let Err(e) = send(&notice) {
        eprintln!("dojo-notify: could not send the notice: {e}");
    }
}

/// `--flag value` pairs and one positional argument.
fn options(args: &[String], flags: &[&str]) -> Result<(Vec<(String, String)>, Vec<String>), String> {
    let (mut found, mut rest) = (Vec::new(), Vec::new());
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if let Some(name) = arg.strip_prefix("--") {
            if !flags.contains(&name) {
                return Err(format!("unknown option --{name}"));
            }
            found.push((name.to_owned(), it.next().ok_or_else(|| format!("--{name} needs a value"))?.clone()));
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((found, rest))
}

fn flag<'a>(found: &'a [(String, String)], name: &str) -> Option<&'a str> {
    found.iter().rev().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

fn send_command(args: &[String]) -> Result<(), String> {
    let (found, rest) = options(args, &["kind", "urgency", "icon", "body", "value", "timeout"])?;
    let [summary] = rest.as_slice() else { return Err("send takes one SUMMARY".into()) };
    let mut notice = Notice::new(flag(&found, "kind").unwrap_or(""), summary.as_str());
    if let Some(u) = flag(&found, "urgency") {
        notice = notice.urgency(u.parse::<Urgency>().map_err(|e| e.to_string())?);
    }
    if let Some(v) = flag(&found, "value") {
        notice = notice.value(v.parse().map_err(|_| "--value is a number from 0 to 100".to_owned())?);
    }
    if let Some(t) = flag(&found, "timeout") {
        notice = notice.timeout_ms(t.parse().map_err(|_| "--timeout is milliseconds".to_owned())?);
    }
    notice = notice.icon(flag(&found, "icon").unwrap_or("")).body(flag(&found, "body").unwrap_or(""));
    send(&notice).map(drop).map_err(|e| e.to_string())
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} {} failed", args.join(" ")));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn volume(args: &[String]) -> Result<(), String> {
    let (found, rest) = options(args, &["step"])?;
    let step: u8 = flag(&found, "step").map_or(Ok(5), str::parse).map_err(|_| "--step is a percentage".to_owned())?;
    match rest.first().map(String::as_str) {
        Some("up") => {
            // Raising the volume unmutes it, as every volume key does.
            run("wpctl", &["set-mute", SINK, "0"])?;
            run("wpctl", &["set-volume", "-l", "1.0", SINK, &format!("{step}%+")])?;
        }
        Some("down") => {
            run("wpctl", &["set-volume", SINK, &format!("{step}%-")])?;
        }
        Some("mute") => {
            run("wpctl", &["set-mute", SINK, "toggle"])?;
        }
        _ => return Err("volume takes up, down or mute".into()),
    }
    let now = level::parse_volume(&run("wpctl", &["get-volume", SINK])?).ok_or("could not read the volume")?;
    announce(volume_notice(now));
    Ok(())
}

fn volume_notice(v: Volume) -> Notice {
    let body = if v.muted { "Muted".to_owned() } else { format!("{}%", v.percent) };
    Notice::new("volume", "Volume")
        .urgency(Urgency::Low)
        .icon(level::volume_icon(v))
        .body(body)
        .value(if v.muted { 0 } else { v.percent })
        .timeout_ms(1500)
}

fn brightness(args: &[String]) -> Result<(), String> {
    let (found, rest) = options(args, &["device", "step"])?;
    let device = flag(&found, "device");
    let step = flag(&found, "step");
    let change = match (rest.first().map(String::as_str), device.is_some_and(|d| d.contains("kbd"))) {
        (Some("up"), keyboard) => format!("{}+", step.map_or(if keyboard { "1" } else { "5%" }, |s| s)),
        (Some("down"), keyboard) => format!("{}-", step.map_or(if keyboard { "1" } else { "5%" }, |s| s)),
        _ => return Err("brightness takes up or down".into()),
    };
    let mut argv = vec!["-m"];
    if let Some(d) = device {
        argv.extend(["--device", d]);
    }
    argv.extend(["set", &change]);
    let percent = level::parse_brightness(&run("brightnessctl", &argv)?).ok_or("could not read the brightness")?;
    let keyboard = device.is_some_and(|d| d.contains("kbd"));
    announce(
        Notice::new(if keyboard { "keyboard-brightness" } else { "brightness" }, if keyboard { "Keyboard backlight" } else { "Brightness" })
            .urgency(Urgency::Low)
            .icon(if keyboard { "keyboard-brightness-symbolic" } else { "display-brightness-symbolic" })
            .body(format!("{percent}%"))
            .value(percent)
            .timeout_ms(1500),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_take_flags_with_values_and_one_summary() {
        let args: Vec<String> = ["--kind", "x", "Hello", "--value", "40"].map(String::from).to_vec();
        let (found, rest) = options(&args, &["kind", "value"]).unwrap();
        assert_eq!((flag(&found, "kind"), flag(&found, "value"), rest), (Some("x"), Some("40"), vec!["Hello".to_owned()]));
        assert!(options(&["--nope".to_owned(), "1".to_owned()], &["kind"]).is_err());
        assert!(options(&["--kind".to_owned()], &["kind"]).is_err());
    }

    #[test]
    fn a_muted_volume_shows_muted_and_an_empty_bar() {
        let n = volume_notice(Volume { percent: 45, muted: true });
        assert_eq!((n.body.as_str(), n.value, n.icon.as_str()), ("Muted", Some(0), "audio-volume-muted-symbolic"));
        let n = volume_notice(Volume { percent: 45, muted: false });
        assert_eq!((n.body.as_str(), n.value, n.kind.as_str()), ("45%", Some(45), "volume"));
    }
}
