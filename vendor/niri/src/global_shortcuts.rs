//! System-wide shortcuts grabbed by IPC clients at runtime.
//!
//! Unlike configured binds, these are owned by an IPC connection and end with it. A grabbed key's
//! press and release are consumed and reported as events, so the focused client never sees them.

use std::collections::HashMap;
use std::str::FromStr;
use std::time::Duration;

use niri_config::{Config, Key, ModKey, Modifiers, Trigger};
use niri_ipc::{Event, GlobalShortcut, GlobalShortcutGrants, RefusedShortcut, Timestamp};
use smithay::backend::input::Keycode;
use smithay::input::keyboard::{keysyms, Keysym, ModifiersState};

use crate::input::modifiers_from_state;
use crate::niri::State;

struct Grab {
    id: String,
    key: Key,
}

/// Grabs by owning connection, and the grabs currently held down.
#[derive(Default)]
pub struct GlobalShortcuts {
    grabs: HashMap<u64, Vec<Grab>>,
    active: HashMap<Keycode, (u64, String)>,
    /// Events sent, for tests: the fixture has no IPC server to read them from.
    #[cfg(test)]
    pub(crate) sent: Vec<Event>,
}

/// The modifiers a bind matches against, with the compositor modifier and `mod_key` made
/// equivalent the way configured binds treat them.
pub(crate) fn effective_modifiers(modifiers: Modifiers, mod_key: ModKey) -> Modifiers {
    let mut modifiers = modifiers;
    if modifiers.contains(Modifiers::COMPOSITOR) {
        modifiers |= mod_key.to_modifiers();
    } else if modifiers.contains(mod_key.to_modifiers()) {
        modifiers |= Modifiers::COMPOSITOR;
    }
    modifiers
}

fn same_key(a: &Key, b: &Key, mod_key: ModKey) -> bool {
    a.trigger == b.trigger
        && effective_modifiers(a.modifiers, mod_key) == effective_modifiers(b.modifiers, mod_key)
}

/// Keys that must stay with the compositor whatever a client asks for.
fn is_hardcoded(trigger: Trigger) -> bool {
    let Trigger::Keysym(keysym) = trigger else {
        return false;
    };
    matches!(
        keysym.raw(),
        keysyms::KEY_XF86Switch_VT_1..=keysyms::KEY_XF86Switch_VT_12 | keysyms::KEY_XF86PowerOff
    )
}

impl GlobalShortcuts {
    /// Replace one connection's grabs and report what could not be grabbed.
    ///
    /// Configured binds always win, so `config_binds` are keys that stay refused.
    fn replace<'a>(
        &mut self,
        owner: u64,
        requested: Vec<GlobalShortcut>,
        mod_key: ModKey,
        config_binds: impl Iterator<Item = &'a Key> + Clone,
    ) -> GlobalShortcutGrants {
        // The old set must not conflict with its own replacement.
        self.grabs.remove(&owner);
        self.active
            .retain(|_, (active_owner, _)| *active_owner != owner);

        let mut grabs: Vec<Grab> = Vec::new();
        let mut refused = Vec::new();
        for GlobalShortcut { id, key } in requested {
            let refuse = |reason: &str| RefusedShortcut {
                id: id.clone(),
                reason: reason.to_owned(),
            };
            let key = match Key::from_str(&key) {
                Ok(key) => key,
                Err(err) => {
                    refused.push(refuse(&format!("invalid key: {err}")));
                    continue;
                }
            };
            if !matches!(key.trigger, Trigger::Keysym(_)) {
                refused.push(refuse("only keyboard keys can be grabbed"));
            } else if is_hardcoded(key.trigger) {
                refused.push(refuse("reserved by the compositor"));
            } else if config_binds
                .clone()
                .any(|bind| same_key(bind, &key, mod_key))
            {
                refused.push(refuse("already a configured bind"));
            } else if grabs.iter().any(|grab| same_key(&grab.key, &key, mod_key)) {
                refused.push(refuse("requested more than once"));
            } else if self
                .grabs
                .values()
                .flatten()
                .any(|grab| same_key(&grab.key, &key, mod_key))
            {
                refused.push(refuse("grabbed by another connection"));
            } else {
                grabs.push(Grab { id, key });
            }
        }

        if !grabs.is_empty() {
            self.grabs.insert(owner, grabs);
        }
        GlobalShortcutGrants { owner, refused }
    }

    fn find(&self, mod_key: ModKey, trigger: Trigger, modifiers: Modifiers) -> Option<(u64, &str)> {
        let mut modifiers = modifiers;
        if modifiers.contains(mod_key.to_modifiers()) {
            modifiers |= Modifiers::COMPOSITOR;
        }
        self.grabs.iter().find_map(|(owner, grabs)| {
            grabs
                .iter()
                .find(|grab| {
                    grab.key.trigger == trigger
                        && effective_modifiers(grab.key.modifiers, mod_key) == modifiers
                })
                .map(|grab| (*owner, grab.id.as_str()))
        })
    }

    /// Drop every grab of a closed connection. Its held keys stay suppressed until released.
    fn remove_owner(&mut self, owner: u64) {
        self.grabs.remove(&owner);
        self.active
            .retain(|_, (active_owner, _)| *active_owner != owner);
    }
}

impl State {
    pub fn set_global_shortcuts(
        &mut self,
        owner: u64,
        shortcuts: Vec<GlobalShortcut>,
    ) -> GlobalShortcutGrants {
        let config = self.niri.config.clone();
        let config = config.borrow();
        let mod_key = self.backend.mod_key(&config);
        self.niri
            .global_shortcuts
            .replace(owner, shortcuts, mod_key, config_keys(&config))
    }

    pub fn release_global_shortcuts(&mut self, owner: u64) {
        self.niri.global_shortcuts.remove_owner(owner);
    }

    /// Whether this press belongs to a grabbed shortcut. If so, report it and consume the key.
    pub(crate) fn global_shortcut_press(
        &mut self,
        key_code: Keycode,
        raw: Option<Keysym>,
        mods: ModifiersState,
        mod_key: ModKey,
        time: Duration,
    ) -> bool {
        if self.niri.is_locked() {
            return false;
        }
        let Some(raw) = raw else {
            return false;
        };
        let shortcuts = &mut self.niri.global_shortcuts;
        let Some((owner, id)) =
            shortcuts.find(mod_key, Trigger::Keysym(raw), modifiers_from_state(mods))
        else {
            return false;
        };
        let id = id.to_owned();
        if shortcuts.active.contains_key(&key_code) {
            return true;
        }
        shortcuts.active.insert(key_code, (owner, id.clone()));
        self.send_global_shortcut_event(Event::ShortcutActivated {
            owner,
            id,
            time: Timestamp::from(time),
        });
        true
    }

    pub(crate) fn global_shortcut_release(&mut self, key_code: Keycode, time: Duration) {
        let Some((owner, id)) = self.niri.global_shortcuts.active.remove(&key_code) else {
            return;
        };
        self.send_global_shortcut_event(Event::ShortcutDeactivated {
            owner,
            id,
            time: Timestamp::from(time),
        });
    }

    fn send_global_shortcut_event(&mut self, event: Event) {
        #[cfg(test)]
        self.niri.global_shortcuts.sent.push(event.clone());
        if let Some(server) = &self.niri.ipc_server {
            server.send_event(event);
        }
    }
}

fn config_keys(config: &Config) -> impl Iterator<Item = &Key> + Clone {
    config
        .binds
        .0
        .iter()
        .chain(config.recent_windows.binds.iter())
        .map(|bind| &bind.key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shortcut(id: &str, key: &str) -> GlobalShortcut {
        GlobalShortcut {
            id: id.to_owned(),
            key: key.to_owned(),
        }
    }

    fn key(s: &str) -> Key {
        Key::from_str(s).unwrap()
    }

    fn replace(
        shortcuts: &mut GlobalShortcuts,
        owner: u64,
        requested: Vec<GlobalShortcut>,
        config: &[Key],
    ) -> GlobalShortcutGrants {
        shortcuts.replace(owner, requested, ModKey::Super, config.iter())
    }

    fn refused(grants: &GlobalShortcutGrants) -> Vec<(&str, &str)> {
        grants
            .refused
            .iter()
            .map(|r| (r.id.as_str(), r.reason.as_str()))
            .collect()
    }

    #[test]
    fn grabs_a_free_key() {
        let mut shortcuts = GlobalShortcuts::default();
        let grants = replace(&mut shortcuts, 1, vec![shortcut("ptt", "Ctrl+Alt+K")], &[]);
        assert_eq!(grants.owner, 1);
        assert!(grants.refused.is_empty());

        let k = key("Ctrl+Alt+K");
        let found = shortcuts.find(ModKey::Super, k.trigger, k.modifiers);
        assert_eq!(found, Some((1, "ptt")));
        let other = key("Ctrl+K");
        assert_eq!(
            shortcuts.find(ModKey::Super, other.trigger, other.modifiers),
            None
        );
    }

    #[test]
    fn refuses_invalid_mouse_and_reserved_keys() {
        let mut shortcuts = GlobalShortcuts::default();
        let grants = replace(
            &mut shortcuts,
            1,
            vec![
                shortcut("bad", "Ctrl+NoSuchKey"),
                shortcut("mouse", "Ctrl+MouseLeft"),
                shortcut("vt", "XF86Switch_VT_2"),
                shortcut("power", "Ctrl+XF86PowerOff"),
            ],
            &[],
        );
        let refused = refused(&grants);
        assert_eq!(refused.len(), 4);
        assert!(refused[0].1.starts_with("invalid key"));
        assert_eq!(refused[1], ("mouse", "only keyboard keys can be grabbed"));
        assert_eq!(refused[2], ("vt", "reserved by the compositor"));
        assert_eq!(refused[3], ("power", "reserved by the compositor"));
        assert!(shortcuts.grabs.is_empty());
    }

    #[test]
    fn configured_binds_win_even_through_the_mod_alias() {
        let mut shortcuts = GlobalShortcuts::default();
        let config = [key("Mod+Shift+P")];
        let grants = replace(
            &mut shortcuts,
            1,
            vec![shortcut("a", "Super+Shift+P"), shortcut("b", "Shift+P")],
            &config,
        );
        assert_eq!(refused(&grants), [("a", "already a configured bind")]);
        assert_eq!(shortcuts.grabs[&1].len(), 1);
    }

    #[test]
    fn first_connection_keeps_a_contested_key() {
        let mut shortcuts = GlobalShortcuts::default();
        replace(&mut shortcuts, 1, vec![shortcut("a", "Ctrl+F1")], &[]);
        let grants = replace(
            &mut shortcuts,
            2,
            vec![shortcut("b", "Ctrl+F1"), shortcut("c", "Ctrl+F1")],
            &[],
        );
        assert_eq!(
            refused(&grants),
            [
                ("b", "grabbed by another connection"),
                ("c", "grabbed by another connection")
            ]
        );
        assert!(!shortcuts.grabs.contains_key(&2));
    }

    #[test]
    fn duplicate_within_one_request_is_refused() {
        let mut shortcuts = GlobalShortcuts::default();
        let grants = replace(
            &mut shortcuts,
            1,
            vec![shortcut("a", "Ctrl+F1"), shortcut("b", "Ctrl+F1")],
            &[],
        );
        assert_eq!(refused(&grants), [("b", "requested more than once")]);
    }

    #[test]
    fn replacing_keeps_a_key_the_connection_already_held() {
        let mut shortcuts = GlobalShortcuts::default();
        replace(&mut shortcuts, 1, vec![shortcut("a", "Ctrl+F1")], &[]);
        shortcuts
            .active
            .insert(Keycode::new(67), (1, "a".to_owned()));
        let grants = replace(
            &mut shortcuts,
            1,
            vec![shortcut("a", "Ctrl+F1"), shortcut("b", "Ctrl+F2")],
            &[],
        );
        assert!(grants.refused.is_empty());
        assert_eq!(shortcuts.grabs[&1].len(), 2);
        // A replaced set forgets held keys: their release is still swallowed by suppression.
        assert!(shortcuts.active.is_empty());
    }

    #[test]
    fn an_empty_set_releases_everything() {
        let mut shortcuts = GlobalShortcuts::default();
        replace(&mut shortcuts, 1, vec![shortcut("a", "Ctrl+F1")], &[]);
        let grants = replace(&mut shortcuts, 1, vec![], &[]);
        assert!(grants.refused.is_empty());
        assert!(shortcuts.grabs.is_empty());
    }

    #[test]
    fn closing_a_connection_frees_its_keys() {
        let mut shortcuts = GlobalShortcuts::default();
        replace(&mut shortcuts, 1, vec![shortcut("a", "Ctrl+F1")], &[]);
        shortcuts
            .active
            .insert(Keycode::new(67), (1, "a".to_owned()));
        shortcuts.remove_owner(1);
        assert!(shortcuts.grabs.is_empty());
        assert!(shortcuts.active.is_empty());
        let grants = replace(&mut shortcuts, 2, vec![shortcut("b", "Ctrl+F1")], &[]);
        assert!(grants.refused.is_empty());
    }

    #[test]
    fn matching_follows_the_compositor_modifier() {
        let mut shortcuts = GlobalShortcuts::default();
        replace(&mut shortcuts, 1, vec![shortcut("a", "Mod+F5")], &[]);
        // With Super as the compositor modifier, a held Super matches `Mod+F5`.
        let held = key("Super+F5");
        assert_eq!(
            shortcuts.find(ModKey::Super, held.trigger, held.modifiers),
            Some((1, "a"))
        );
        // With Alt as the compositor modifier it does not.
        assert_eq!(
            shortcuts.find(ModKey::Alt, held.trigger, held.modifiers),
            None
        );
    }
}
