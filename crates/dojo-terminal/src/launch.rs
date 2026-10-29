//! The program a new session or pane runs. The Rex server is a systemd user service that may
//! predate this desktop session, so the desktop's own variables go in through `env`.
use std::path::PathBuf;

/// The same list the shell passes to the quick terminal's session.
const SESSION_VARIABLES: [&str; 7] = [
    "WAYLAND_DISPLAY",
    "NIRI_SOCKET",
    "DISPLAY",
    "DBUS_SESSION_BUS_ADDRESS",
    "SSH_AUTH_SOCK",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_TYPE",
];

/// `env VARIABLES... SHELL`, or nothing (the login shell) when no shell is chosen.
pub fn shell_command(var: impl Fn(&str) -> Option<String>, exists: impl Fn(&PathBuf) -> bool) -> Vec<String> {
    let shell = var("DOJO_TERMINAL_SHELL").filter(|shell| !shell.is_empty()).or_else(|| {
        let rush = PathBuf::from(var("HOME")?).join(".nix-profile/bin/rush");
        exists(&rush).then(|| rush.display().to_string())
    });
    let Some(shell) = shell else { return Vec::new() };
    let mut command = vec!["env".to_owned()];
    command.extend(SESSION_VARIABLES.iter().filter_map(|name| var(name).map(|value| format!("{name}={value}"))));
    command.push(shell);
    command
}

pub fn from_environment() -> Vec<String> {
    shell_command(|name| std::env::var(name).ok(), |path| path.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(name: &str) -> Option<String> {
        match name {
            "HOME" => Some("/home/u".into()),
            "WAYLAND_DISPLAY" => Some("wayland-1".into()),
            "NIRI_SOCKET" => Some("/run/niri".into()),
            _ => None,
        }
    }

    #[test]
    fn rush_runs_through_env_with_the_desktop_variables() {
        let command = shell_command(vars, |_| true);
        assert_eq!(command, ["env", "WAYLAND_DISPLAY=wayland-1", "NIRI_SOCKET=/run/niri", "/home/u/.nix-profile/bin/rush"]);
    }

    #[test]
    fn the_shell_variable_wins_and_no_shell_means_the_login_shell() {
        let with = |name: &str| if name == "DOJO_TERMINAL_SHELL" { Some("/bin/fish".into()) } else { vars(name) };
        assert_eq!(shell_command(with, |_| false).last().map(String::as_str), Some("/bin/fish"));
        assert!(shell_command(vars, |_| false).is_empty());
    }
}
