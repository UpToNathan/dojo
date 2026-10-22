//! A shortcut's key combination, converted between the portal's XDG syntax, Niri's key syntax and
//! the key names a GPUI keystroke carries.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A key and the modifiers held with it. The key is an xkb keysym name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Trigger {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub logo: bool,
    key: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TriggerError(String);

impl fmt::Display for TriggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TriggerError {}

fn error(message: impl Into<String>) -> TriggerError {
    TriggerError(message.into())
}

impl Trigger {
    pub fn new(
        ctrl: bool,
        alt: bool,
        shift: bool,
        logo: bool,
        key: &str,
    ) -> Result<Self, TriggerError> {
        if key.is_empty() {
            return Err(error("a shortcut needs a key"));
        }
        if !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return Err(error(format!("{key:?} is not a key name")));
        }
        if modifier_only(key) {
            return Err(error("a modifier alone cannot be a shortcut"));
        }
        // A letter is the same key whatever its case; keep one spelling so equal triggers compare equal.
        let key = if key.len() == 1 {
            key.to_ascii_uppercase()
        } else {
            key.to_owned()
        };
        Ok(Self {
            ctrl,
            alt,
            shift,
            logo,
            key,
        })
    }

    pub fn key(&self) -> &str {
        &self.key
    }

    /// Parse the portal's trigger syntax: `CTRL+ALT+a`, `LOGO+space`, `SHIFT+F5`.
    pub fn from_xdg(text: &str) -> Result<Self, TriggerError> {
        let mut parts = text.split('+');
        let key = parts.next_back().unwrap_or_default().trim();
        let (mut ctrl, mut alt, mut shift, mut logo) = (false, false, false, false);
        for part in parts {
            let part = part.trim();
            match part.to_ascii_uppercase().as_str() {
                "CTRL" | "CONTROL" => ctrl = true,
                "ALT" => alt = true,
                "SHIFT" => shift = true,
                "LOGO" | "SUPER" | "META" => logo = true,
                "NUM" => return Err(error("Num Lock triggers are not supported")),
                _ => return Err(error(format!("unknown modifier {part:?}"))),
            }
        }
        Self::new(ctrl, alt, shift, logo, key)
    }

    /// Build a trigger from a GPUI keystroke's key name.
    pub fn from_keystroke(
        ctrl: bool,
        alt: bool,
        shift: bool,
        logo: bool,
        key: &str,
    ) -> Result<Self, TriggerError> {
        let name = match key {
            "space" => "space",
            "enter" => "Return",
            "escape" => "Escape",
            "tab" => "Tab",
            "backspace" => "BackSpace",
            "delete" => "Delete",
            "insert" => "Insert",
            "home" => "Home",
            "end" => "End",
            "pageup" => "Prior",
            "pagedown" => "Next",
            "up" => "Up",
            "down" => "Down",
            "left" => "Left",
            "right" => "Right",
            other => other,
        };
        let name = match name.strip_prefix('f') {
            Some(number) if number.parse::<u8>().is_ok() => format!("F{number}"),
            _ => name.to_owned(),
        };
        Self::new(ctrl, alt, shift, logo, &name)
    }

    /// Niri's key syntax, as `Request::GlobalShortcuts` takes it.
    pub fn to_niri(&self) -> String {
        self.text("Super")
    }

    /// What to show the user, and what the portal reports as `trigger_description`.
    pub fn description(&self) -> String {
        self.text("Super")
    }

    fn text(&self, logo: &str) -> String {
        let mut text = String::new();
        for (held, name) in [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.logo, logo),
        ] {
            if held {
                text.push_str(name);
                text.push('+');
            }
        }
        text.push_str(&self.key);
        text
    }
}

fn modifier_only(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    ["shift", "control", "ctrl", "alt", "super", "meta", "hyper", "caps_lock", "num_lock"]
        .iter()
        .any(|name| {
            let name = *name;
            lower == name
                || lower.strip_suffix("_l") == Some(name)
                || lower.strip_suffix("_r") == Some(name)
        })
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}

impl TryFrom<String> for Trigger {
    type Error = TriggerError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        Self::from_xdg(&text)
    }
}

impl From<Trigger> for String {
    fn from(trigger: Trigger) -> Self {
        trigger.description()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_portal_syntax() {
        let trigger = Trigger::from_xdg("CTRL+ALT+a").unwrap();
        assert!(trigger.ctrl && trigger.alt && !trigger.shift && !trigger.logo);
        assert_eq!(trigger.key(), "A");
        assert_eq!(trigger.to_niri(), "Ctrl+Alt+A");

        let trigger = Trigger::from_xdg("LOGO+space").unwrap();
        assert_eq!(trigger.to_niri(), "Super+space");
        assert_eq!(Trigger::from_xdg("shift+F5").unwrap().to_niri(), "Shift+F5");
        assert_eq!(Trigger::from_xdg("F13").unwrap().to_niri(), "F13");
    }

    #[test]
    fn spelling_does_not_change_identity() {
        assert_eq!(
            Trigger::from_xdg("Ctrl+Alt+k").unwrap(),
            Trigger::from_xdg("alt+control+K").unwrap()
        );
    }

    #[test]
    fn rejects_what_niri_could_not_grab() {
        for bad in [
            "",
            "CTRL+",
            "CTRL+ALT",
            "HYPER+a",
            "NUM+a",
            "Shift",
            "CTRL+Shift_L",
            "CTRL+a b",
            "CTRL+++",
        ] {
            assert!(Trigger::from_xdg(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn maps_gpui_key_names_to_keysym_names() {
        let t = |key| Trigger::from_keystroke(true, false, false, false, key).unwrap();
        assert_eq!(t("k").to_niri(), "Ctrl+K");
        assert_eq!(t("f5").to_niri(), "Ctrl+F5");
        assert_eq!(t("f13").to_niri(), "Ctrl+F13");
        assert_eq!(t("enter").to_niri(), "Ctrl+Return");
        assert_eq!(t("pageup").to_niri(), "Ctrl+Prior");
        assert_eq!(t("space").to_niri(), "Ctrl+space");
        assert!(Trigger::from_keystroke(true, false, false, false, "shift").is_err());
        assert!(Trigger::from_keystroke(false, false, false, false, "").is_err());
    }

    #[test]
    fn round_trips_through_text() {
        let trigger = Trigger::new(true, true, true, true, "F12").unwrap();
        assert_eq!(Trigger::from_xdg(&trigger.description()).unwrap(), trigger);
        let toml = toml::to_string(&std::collections::BTreeMap::from([("t", &trigger)])).unwrap();
        assert_eq!(toml.trim(), r#"t = "Ctrl+Alt+Shift+Super+F12""#);
    }
}
