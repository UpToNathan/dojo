//! PAM vocabulary in domain terms. Raw PAM integers are read once here and in
//! `worker::libpam`; nothing past this module sees one except `PamOutcome::Other`.

use std::time::Duration;

use super::worker::libpam as raw;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Method {
    Fingerprint,
    Password,
}

impl Method {
    pub const ALL: [Method; 2] = [Method::Fingerprint, Method::Password];

    /// The PAM service file, and the only place these names appear.
    pub const fn service(self) -> &'static str {
        match self {
            Method::Fingerprint => "dojo-lock-fingerprint",
            Method::Password => "dojo-lock-password",
        }
    }

    pub fn from_service(name: &str) -> Option<Method> {
        Method::ALL
            .into_iter()
            .find(|method| method.service() == name)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WorkerId {
    pub method: Method,
    pub generation: u32,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Report {
    Prompt {
        echo: bool,
    },
    Notice(Notice),
    Finished(PamOutcome),
    /// The worker went away without a result.
    Crashed,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Notice {
    Hint {
        text: String,
        error: bool,
    },
    /// pam_faillock refused the account. It always precedes `PasswordPaused`
    /// unless faillock is configured with `unlock_time=0`, which never unpauses.
    PasswordLocked,
    /// pam_faillock's remaining lock time, rounded up to whole minutes by faillock.
    PasswordPaused {
        remaining: Duration,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum MessageStyle {
    Info,
    Error,
}

impl Notice {
    /// Expects the worker to run with `LC_MESSAGES=C`, so faillock speaks English.
    pub(crate) fn parse(style: MessageStyle, text: &str) -> Notice {
        if text.starts_with("The account is locked due to ") && text.ends_with(" failed logins.") {
            return Notice::PasswordLocked;
        }
        if let Some(minutes) = faillock_minutes(text) {
            return Notice::PasswordPaused {
                remaining: Duration::from_secs(minutes * 60),
            };
        }
        Notice::Hint {
            text: text.to_owned(),
            error: style == MessageStyle::Error,
        }
    }
}

/// "(10 minutes left to unlock)" or "(1 minute left to unlock)".
fn faillock_minutes(text: &str) -> Option<u64> {
    let inner = text.strip_prefix('(')?.strip_suffix(" left to unlock)")?;
    let (number, unit) = inner.split_once(' ')?;
    if unit != "minutes" && unit != "minute" {
        return None;
    }
    number.parse().ok()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PamOutcome {
    Success,
    Rejected,
    Unavailable,
    MaxTries,
    Other(i32),
}

impl PamOutcome {
    pub(crate) fn from_code(code: i32) -> PamOutcome {
        match code {
            raw::PAM_SUCCESS => PamOutcome::Success,
            raw::PAM_AUTH_ERR => PamOutcome::Rejected,
            raw::PAM_AUTHINFO_UNAVAIL => PamOutcome::Unavailable,
            raw::PAM_MAXTRIES => PamOutcome::MaxTries,
            other => PamOutcome::Other(other),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_messages_seen_by_the_spike_are_plain_hints() {
        assert_eq!(
            Notice::parse(
                MessageStyle::Info,
                "Place your right index finger on the fingerprint reader"
            ),
            Notice::Hint {
                text: "Place your right index finger on the fingerprint reader".into(),
                error: false
            }
        );
        assert_eq!(
            Notice::parse(MessageStyle::Info, "Verification timed out"),
            Notice::Hint {
                text: "Verification timed out".into(),
                error: false
            }
        );
        assert_eq!(
            Notice::parse(MessageStyle::Error, "Failed to match fingerprint"),
            Notice::Hint {
                text: "Failed to match fingerprint".into(),
                error: true
            }
        );
    }

    #[test]
    fn faillock_lock_messages_observed_on_this_host_pause_the_password() {
        assert_eq!(
            Notice::parse(
                MessageStyle::Info,
                "The account is locked due to 3 failed logins."
            ),
            Notice::PasswordLocked
        );
        assert_eq!(
            Notice::parse(MessageStyle::Info, "(10 minutes left to unlock)"),
            Notice::PasswordPaused {
                remaining: Duration::from_secs(600)
            }
        );
        assert_eq!(
            Notice::parse(MessageStyle::Info, "(1 minute left to unlock)"),
            Notice::PasswordPaused {
                remaining: Duration::from_secs(60)
            }
        );
    }

    #[test]
    fn near_misses_of_faillock_text_stay_hints() {
        for text in [
            "(ten minutes left to unlock)",
            "(10 hours left to unlock)",
            "10 minutes left to unlock",
            "The account is locked",
        ] {
            assert_eq!(
                Notice::parse(MessageStyle::Info, text),
                Notice::Hint {
                    text: text.into(),
                    error: false
                }
            );
        }
    }

    #[test]
    fn pam_codes_map_to_outcomes() {
        assert_eq!(PamOutcome::from_code(0), PamOutcome::Success);
        assert_eq!(PamOutcome::from_code(7), PamOutcome::Rejected);
        assert_eq!(PamOutcome::from_code(9), PamOutcome::Unavailable);
        assert_eq!(PamOutcome::from_code(11), PamOutcome::MaxTries);
        assert_eq!(PamOutcome::from_code(19), PamOutcome::Other(19));
        assert_eq!(PamOutcome::from_code(-1), PamOutcome::Other(-1));
    }

    #[test]
    fn services_round_trip_and_reject_strangers() {
        assert_eq!(
            Method::from_service("dojo-lock-password"),
            Some(Method::Password)
        );
        assert_eq!(
            Method::from_service("dojo-lock-fingerprint"),
            Some(Method::Fingerprint)
        );
        assert_eq!(Method::from_service("system-auth"), None);
        assert_eq!(Method::from_service("../dojo-lock-password"), None);
    }

    #[test]
    fn every_service_has_a_tracked_pam_file() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("system-config/pam.d");
        for method in Method::ALL {
            let file = dir.join(method.service());
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            assert!(
                text.lines().any(|line| line.starts_with("auth ")),
                "{} has no auth stack",
                file.display()
            );
        }
    }
}
