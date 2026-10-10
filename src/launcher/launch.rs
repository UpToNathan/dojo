use std::collections::HashMap;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, bail};
use freedesktop_desktop_entry::DesktopEntry;

#[derive(Debug, PartialEq, Eq)]
struct ExecPlan {
    argv: Vec<String>,
    directory: Option<PathBuf>,
    terminal: bool,
}

fn desktop_unescape(value: &str) -> String {
    let mut output = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            output.push(ch);
            continue;
        }
        match chars.next() {
            Some('s') => output.push(' '),
            Some('n') => output.push('\n'),
            Some('t') => output.push('\t'),
            Some('r') => output.push('\r'),
            Some('\\') => output.push('\\'),
            Some(next) => {
                output.push('\\');
                output.push(next);
            }
            None => output.push('\\'),
        }
    }
    output
}

fn tokenize(exec: &str) -> anyhow::Result<Vec<String>> {
    let decoded = desktop_unescape(exec);
    let mut chars = decoded.chars();
    let mut arguments = Vec::new();
    let mut argument = String::new();
    let mut quoted = false;
    let mut started = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\0' => bail!("Exec cannot contain NUL"),
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' => {
                let next = chars.next().context("trailing escape in Exec")?;
                if quoted && !matches!(next, '"' | '\\' | '$' | '`') {
                    argument.push('\\');
                }
                argument.push(next);
                started = true;
            }
            ch if ch.is_ascii_whitespace() && !quoted => {
                if started {
                    arguments.push(std::mem::take(&mut argument));
                    started = false;
                }
            }
            other => {
                argument.push(other);
                started = true;
            }
        }
    }
    anyhow::ensure!(!quoted, "unmatched quote in Exec");
    if started {
        arguments.push(argument);
    }
    Ok(arguments)
}

fn exec_plan(entry: &DesktopEntry, name: &str) -> anyhow::Result<ExecPlan> {
    let words = tokenize(entry.exec().context("desktop entry has no Exec command")?)?;
    let mut argv = Vec::new();
    for word in words {
        match word.as_str() {
            "%f" | "%F" | "%u" | "%U" | "%d" | "%D" | "%n" | "%N" | "%v" | "%m" => continue,
            "%i" => {
                if let Some(icon) = entry.icon() {
                    argv.extend(["--icon".into(), desktop_unescape(icon)]);
                }
                continue;
            }
            _ => {}
        }
        let mut expanded = String::new();
        let mut chars = word.chars();
        while let Some(ch) = chars.next() {
            if ch != '%' {
                expanded.push(ch);
                continue;
            }
            match chars.next().context("incomplete Exec field code")? {
                '%' => expanded.push('%'),
                'c' => expanded.push_str(name),
                'k' => expanded.push_str(&entry.path.to_string_lossy()),
                code => bail!("unsupported or embedded Exec field code %{code}"),
            }
        }
        argv.push(expanded);
    }
    let program = argv
        .first()
        .filter(|s| !s.is_empty())
        .context("Exec command is empty")?;
    anyhow::ensure!(
        !program.contains('='),
        "Exec executable cannot be an environment assignment; use env explicitly"
    );
    let directory = entry
        .path()
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(desktop_unescape(s)));
    if let Some(path) = &directory {
        anyhow::ensure!(
            path.is_absolute() && path.is_dir(),
            "desktop working directory is invalid: {}",
            path.display()
        );
    }
    Ok(ExecPlan {
        argv,
        directory,
        terminal: entry.terminal(),
    })
}

fn session_environment(
    connection: &zbus::blocking::Connection,
) -> anyhow::Result<HashMap<String, String>> {
    let manager = zbus::blocking::Proxy::new(
        connection,
        "org.freedesktop.systemd1",
        "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager",
    )?;
    let assignments: Vec<String> = manager.get_property("Environment")?;
    Ok(assignments
        .into_iter()
        .filter_map(|value| {
            value
                .split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect())
}

fn activate(
    entry: &DesktopEntry,
    connection: &zbus::blocking::Connection,
    environment: &HashMap<String, String>,
) -> anyhow::Result<()> {
    let name = entry
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .context("D-Bus app needs a valid desktop filename")?;
    let destination =
        zbus::names::BusName::try_from(name).context("invalid application D-Bus name")?;
    let object = format!("/{}", name.replace('.', "/").replace('-', "_"));
    let proxy = zbus::blocking::Proxy::new(
        connection,
        destination,
        object,
        "org.freedesktop.Application",
    )?;
    let mut platform = HashMap::<&str, zbus::zvariant::Value>::new();
    for (key, var) in [
        ("activation-token", "XDG_ACTIVATION_TOKEN"),
        ("desktop-startup-id", "DESKTOP_STARTUP_ID"),
    ] {
        if let Some(value) = environment.get(var) {
            platform.insert(key, zbus::zvariant::Value::from(value.as_str()));
        }
    }
    let _: () = proxy.call("Activate", &(platform,))?;
    Ok(())
}

fn absent_activation(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<zbus::Error>() {
        Some(zbus::Error::MethodError(name, _, _)) => matches!(
            name.as_str(),
            "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.NameHasNoOwner"
        ),
        _ => false,
    }
}

fn terminal_argv(
    plan: &ExecPlan,
    environment: &HashMap<String, String>,
) -> anyhow::Result<Vec<String>> {
    let terminal = environment
        .get("TERMINAL")
        .map(String::as_str)
        .unwrap_or("kitty");
    let mut argv = tokenize(terminal)?;
    let program = argv.first().context("TERMINAL is empty")?;
    let base = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    match base {
        "kitty" => argv.push("--".into()),
        "ghostty" | "xterm" | "alacritty" | "foot" => argv.push("-e".into()),
        "xdg-terminal-exec" => {}
        _ => bail!(
            "unsupported terminal launch convention for {base}; set TERMINAL to a supported terminal"
        ),
    };
    argv.extend(plan.argv.clone());
    Ok(argv)
}

pub(super) fn launch(path: &Path, name: &str) -> anyhow::Result<()> {
    let locales = freedesktop_desktop_entry::get_languages_from_env();
    let entry = DesktopEntry::from_path(path, Some(&locales))?;
    let connection = zbus::blocking::connection::Builder::session()?
        .method_timeout(std::time::Duration::from_secs(3))
        .build()?;
    let environment = session_environment(&connection)?;
    if entry.dbus_activatable() {
        match activate(&entry, &connection, &environment) {
            Ok(()) => return Ok(()),
            Err(error) if entry.exec().is_some() && absent_activation(&error) => {}
            Err(error) => {
                return Err(error)
                    .context("application activation failed; not executing a duplicate fallback");
            }
        }
    }
    let plan = exec_plan(&entry, name)?;
    let active = Command::new("uwsm")
        .args(["check", "is-active"])
        .env_clear()
        .envs(&environment)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    let mut command = if active {
        let mut command = Command::new("uwsm");
        command.args(["app", "-t", "service", "-d", name]);
        if let Some(directory) = &plan.directory {
            command.current_dir(directory);
        }
        if plan.terminal {
            command.arg("-T");
        }
        command.arg("--").args(&plan.argv);
        command
    } else {
        let argv = if plan.terminal {
            terminal_argv(&plan, &environment)?
        } else {
            plan.argv.clone()
        };
        let mut command = Command::new("systemd-run");
        command.args([
            "--user",
            "--scope",
            "--collect",
            "--quiet",
            "--slice=app.slice",
        ]);
        if let Some(directory) = &plan.directory {
            command.current_dir(directory);
        }
        command
            .args(["--", "systemd-cat", "--identifier=dojo-app-launch"])
            .args(argv);
        command
    };
    let status = command
        .env_clear()
        .envs(environment)
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .status()?;
    anyhow::ensure!(
        status.success(),
        "launch failed ({status}); see application journal logs"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(exec: &str) -> DesktopEntry {
        DesktopEntry::from_str(
            PathBuf::from("/tmp/org.dojo.Test.desktop"),
            &format!(
                "[Desktop Entry]\nType=Application\nName=Test Name\nIcon=test-icon\nExec={exec}\n"
            ),
            None::<&[&str]>,
        )
        .expect("valid test fixture")
    }

    #[test]
    fn quoted_executable_and_arguments_are_not_shell_split() -> anyhow::Result<()> {
        let plan = exec_plan(
            &entry(r#""/path with spaces/app" %c %% "two words" "" %U %i %k"#),
            "Test Name",
        )?;
        assert_eq!(
            plan.argv,
            [
                "/path with spaces/app",
                "Test Name",
                "%",
                "two words",
                "",
                "--icon",
                "test-icon",
                "/tmp/org.dojo.Test.desktop"
            ]
        );
        Ok(())
    }
    #[test]
    fn shell_metacharacters_remain_literal_and_invalid_exec_is_rejected() -> anyhow::Result<()> {
        let plan = exec_plan(&entry(r#"app "$HOME; $(touch nope)""#), "Test")?;
        assert_eq!(plan.argv, ["app", "$HOME; $(touch nope)"]);
        for invalid in [
            r#"app "unfinished"#,
            "app %z",
            "",
            "FOO=bar app",
            "app %i-suffix",
        ] {
            assert!(
                exec_plan(&entry(invalid), "Test").is_err(),
                "accepted {invalid}"
            );
        }
        Ok(())
    }
}
