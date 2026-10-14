use crate::EmbeddedEnvironmentVariable;
use std::collections::BTreeMap;
const KEY_GAP_ENV: &str = "CUA_DRIVER_KEY_GAP_MS";
const WINDOW_CHANGE_TIMEOUT_ENV: &str = "CUA_DRIVER_WINDOW_CHANGE_TIMEOUT_MS";
const WINDOW_CHANGE_POLL_ENV: &str = "CUA_DRIVER_WINDOW_CHANGE_POLL_MS";

pub(crate) fn allowed_environment_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("LC_")
        || upper == WINDOW_CHANGE_TIMEOUT_ENV
        || upper == WINDOW_CHANGE_POLL_ENV
        || upper == KEY_GAP_ENV
        || matches!(
            upper.as_str(),
            "PATH"
                | "HOME"
                | "USER"
                | "LOGNAME"
                | "SHELL"
                | "TMPDIR"
                | "TMP"
                | "TEMP"
                | "LANG"
                | "SYSTEMROOT"
                | "WINDIR"
                | "COMSPEC"
                | "PATHEXT"
                | "APPDATA"
                | "LOCALAPPDATA"
                | "PROGRAMDATA"
                | "DISPLAY"
                | "WAYLAND_DISPLAY"
                | "LD_LIBRARY_PATH"
                | "NIRI_SOCKET"
                | "CUA_DRIVER_RS_ENABLE_WAYLAND"
                | "XDG_RUNTIME_DIR"
                | "XDG_SESSION_TYPE"
                | "DBUS_SESSION_BUS_ADDRESS"
                | "XAUTHORITY"
                | "CUA_LOG"
                | "CUA_DRIVER_RS_TELEMETRY_ENABLED"
                | "CUA_TELEMETRY_ENABLED"
                | "CUA_DRIVER_RS_UPDATE_CHECK"
        )
}

pub(crate) fn inherited_managed_environment_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "CUA_DRIVER_PERMISSION_MODE"
            | "CUA_DRIVER_DANGEROUSLY_BYPASS_APPROVALS"
            | "CUA_DRIVER_DISABLE_UNRESTRICTED"
            | "CUA_DRIVER_SESSION_POLICY_FILE"
            | "CUA_DRIVER_SESSION_POLICY_APPROVED"
            | "CUA_DRIVER_CAPABILITY_MANIFEST_FILE"
            | "CUA_DRIVER_CAPABILITY_MANIFEST_APPROVED"
            | "CUA_DRIVER_POLICY_FILE"
            | "CUA_DRIVER_MANAGED_POLICY_FILE"
    )
}

pub(crate) fn safe_environment(
    overrides: &[EmbeddedEnvironmentVariable],
) -> Vec<EmbeddedEnvironmentVariable> {
    merge_safe_environment(std::env::vars(), overrides)
}

fn merge_safe_environment(
    inherited: impl IntoIterator<Item = (String, String)>,
    overrides: &[EmbeddedEnvironmentVariable],
) -> Vec<EmbeddedEnvironmentVariable> {
    let mut values = BTreeMap::new();
    for (name, value) in inherited {
        if allowed_environment_name(&name) || inherited_managed_environment_name(&name) {
            let canonical_name = if inherited_managed_environment_name(&name) {
                name.to_ascii_uppercase()
            } else {
                name
            };
            values.insert(
                canonical_name.to_ascii_uppercase(),
                EmbeddedEnvironmentVariable {
                    name: canonical_name,
                    value,
                },
            );
        }
    }
    for variable in overrides {
        if allowed_environment_name(&variable.name) {
            values.insert(variable.name.to_ascii_uppercase(), variable.clone());
        }
    }
    values.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_keeps_native_endpoint_and_managed_policy_without_provider_keys() {
        let variables = merge_safe_environment(
            [
                ("OPENAI_API_KEY".into(), "fake-secret".into()),
                ("ANTHROPIC_API_KEY".into(), "fake-secret".into()),
                ("NIRI_SOCKET".into(), "/run/user/1000/niri.sock".into()),
                ("CUA_DRIVER_PERMISSION_MODE".into(), "bounded".into()),
            ],
            &[EmbeddedEnvironmentVariable {
                name: "CUA_DRIVER_PERMISSION_MODE".into(),
                value: "unrestricted".into(),
            }],
        );
        assert_eq!(
            variables,
            vec![
                EmbeddedEnvironmentVariable {
                    name: "CUA_DRIVER_PERMISSION_MODE".into(),
                    value: "bounded".into()
                },
                EmbeddedEnvironmentVariable {
                    name: "NIRI_SOCKET".into(),
                    value: "/run/user/1000/niri.sock".into()
                },
            ]
        );
    }
}
