//! Per-feature switches from the `[session]` table of `~/.config/dojo/config.toml`.
//!
//! The table is parsed with the rest of the config in `config.rs`; this type is
//! the validated result that the shell and the locker read.

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct SessionSettings {
    /// Play the session transition. Off means the frame changes state at once.
    pub transition: bool,
    pub greeting: bool,
    pub farewell: bool,
    /// Cross-fade instead of moving. GPUI on Linux never reports a desktop
    /// setting, so this is the only way to reach reduced motion in the locker.
    pub reduced_motion: bool,
}

impl Default for SessionSettings {
    fn default() -> Self {
        Self {
            transition: true,
            greeting: true,
            farewell: true,
            reduced_motion: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::parse_session_settings;

    use super::*;

    #[test]
    fn everything_is_on_by_default() {
        let all_on = SessionSettings {
            transition: true,
            greeting: true,
            farewell: true,
            reduced_motion: false,
        };
        assert_eq!(SessionSettings::default(), all_on);
        assert_eq!(parse_session_settings("").unwrap(), all_on);
        assert_eq!(parse_session_settings("[session]").unwrap(), all_on);
        assert_eq!(
            parse_session_settings("[theme.colors]\nbackground = '#123456'").unwrap(),
            all_on
        );
    }

    #[test]
    fn each_switch_turns_off_independently() {
        let parse =
            |key: &str| parse_session_settings(&format!("[session]\n{key} = false")).unwrap();
        assert_eq!(
            parse("transition"),
            SessionSettings {
                transition: false,
                ..Default::default()
            }
        );
        assert_eq!(
            parse("greeting"),
            SessionSettings {
                greeting: false,
                ..Default::default()
            }
        );
        assert_eq!(
            parse("farewell"),
            SessionSettings {
                farewell: false,
                ..Default::default()
            }
        );
    }

    #[test]
    fn reduced_motion_is_off_by_default_and_turns_on_with_its_switch() {
        assert!(!parse_session_settings("[session]").unwrap().reduced_motion);
        let on = parse_session_settings("[session]\nreduced_motion = true").unwrap();
        assert_eq!(
            on,
            SessionSettings {
                reduced_motion: true,
                ..Default::default()
            }
        );
        assert!(parse_session_settings("[session]\nreduced_motion = 'yes'").is_err());
    }

    #[test]
    fn switches_coexist_with_theme_colors() {
        let settings = parse_session_settings(
            "[theme.colors]\nbackground = '#123456'\n[session]\ngreeting = false\nfarewell = true",
        )
        .unwrap();
        assert_eq!(
            settings,
            SessionSettings {
                greeting: false,
                ..Default::default()
            }
        );
    }

    #[test]
    fn unknown_keys_and_wrong_types_are_rejected() {
        for invalid in [
            "[session]\ngreting = false",
            "[session]\ngreeting = 'no'",
            "[session]\ntransition = 0",
        ] {
            assert!(
                parse_session_settings(invalid).is_err(),
                "accepted {invalid}"
            );
        }
    }
}
