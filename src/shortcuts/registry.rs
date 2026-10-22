//! Sessions, their shortcuts and the triggers chosen for them. Pure state: the portal service
//! drives it and carries what it returns to Niri, the disk and D-Bus.

use std::collections::BTreeMap;

use super::store::{Assignment, Assignments};
use super::trigger::Trigger;

/// Apps that reach the portal without an app id (anything outside Flatpak that has not
/// registered one) share this bucket.
const UNREGISTERED: &str = "(unregistered)";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistryError {
    UnknownSession,
    DuplicateSession,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::UnknownSession => "no such shortcuts session",
            Self::DuplicateSession => "shortcuts session already exists",
        })
    }
}

impl std::error::Error for RegistryError {}

/// A shortcut an app asked to bind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Requested {
    pub id: String,
    pub description: String,
    /// The app's suggestion, unparsed: only a hint, and sometimes not a valid trigger.
    pub preferred_trigger: Option<String>,
}

/// A shortcut the user is asked about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pending {
    pub id: String,
    pub description: String,
    pub preferred: Option<Trigger>,
    pub current: Option<Trigger>,
}

/// The user's answer for one shortcut. `None` declines it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub trigger: Option<Trigger>,
}

/// One shortcut as reported to the app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    pub id: String,
    pub description: String,
    /// `None` when unbound, declined or refused by the compositor.
    pub trigger: Option<Trigger>,
}

/// One key for the compositor to grab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grab {
    pub id: String,
    pub key: String,
}

struct Entry {
    id: String,
    description: String,
    trigger: Option<Trigger>,
    refused: Option<String>,
}

struct Session {
    serial: u64,
    app_key: String,
    /// Whether the app has called `BindShortcuts` in this session.
    bound: bool,
    shortcuts: Vec<Entry>,
}

#[derive(Default)]
pub struct Registry {
    assignments: Assignments,
    sessions: BTreeMap<String, Session>,
    next_serial: u64,
}

fn app_key(app_id: &str) -> String {
    if app_id.is_empty() {
        UNREGISTERED.to_owned()
    } else {
        app_id.to_owned()
    }
}

impl Registry {
    pub fn new(assignments: Assignments) -> Self {
        Self {
            assignments,
            ..Self::default()
        }
    }

    pub fn assignments(&self) -> &Assignments {
        &self.assignments
    }

    pub fn create_session(&mut self, handle: &str, app_id: &str) -> Result<(), RegistryError> {
        if self.sessions.contains_key(handle) {
            return Err(RegistryError::DuplicateSession);
        }
        self.next_serial += 1;
        self.sessions.insert(
            handle.to_owned(),
            Session {
                serial: self.next_serial,
                app_key: app_key(app_id),
                bound: false,
                shortcuts: Vec::new(),
            },
        );
        Ok(())
    }

    /// Forget every session, as when the portal stops.
    pub fn close_all(&mut self) {
        self.sessions.clear();
    }

    pub fn close_session(&mut self, handle: &str) -> bool {
        self.sessions.remove(handle).is_some()
    }

    pub fn app_id(&self, handle: &str) -> Result<&str, RegistryError> {
        let session = self.session(handle)?;
        Ok(if session.app_key == UNREGISTERED {
            ""
        } else {
            &session.app_key
        })
    }

    fn session(&self, handle: &str) -> Result<&Session, RegistryError> {
        self.sessions.get(handle).ok_or(RegistryError::UnknownSession)
    }

    /// The requested shortcuts the user has never been asked about for this app.
    pub fn plan_bind(
        &self,
        handle: &str,
        requested: &[Requested],
    ) -> Result<Vec<Pending>, RegistryError> {
        let session = self.session(handle)?;
        let known = self.assignments.apps.get(&session.app_key);
        Ok(requested
            .iter()
            .filter(|request| !known.is_some_and(|known| known.contains_key(&request.id)))
            .map(|request| Pending {
                id: request.id.clone(),
                description: request.description.clone(),
                preferred: request
                    .preferred_trigger
                    .as_deref()
                    .and_then(|text| Trigger::from_xdg(text).ok()),
                current: None,
            })
            .collect())
    }

    /// Bind the requested shortcuts with the user's choices, falling back to what was chosen
    /// before. Returns whether the stored choices changed.
    pub fn commit_bind(
        &mut self,
        handle: &str,
        requested: &[Requested],
        choices: &[Choice],
    ) -> Result<bool, RegistryError> {
        let app_key = self.session(handle)?.app_key.clone();
        let stored = self.assignments.apps.entry(app_key).or_default();
        let mut changed = false;
        let mut bound = Vec::new();
        for request in requested {
            let chosen = choices.iter().find(|choice| choice.id == request.id);
            let assignment = match (chosen, stored.get(&request.id)) {
                (Some(choice), _) => Assignment {
                    trigger: choice.trigger.clone(),
                    description: request.description.clone(),
                },
                (None, Some(previous)) => Assignment {
                    trigger: previous.trigger.clone(),
                    description: request.description.clone(),
                },
                (None, None) => Assignment {
                    trigger: None,
                    description: request.description.clone(),
                },
            };
            if stored.get(&request.id) != Some(&assignment) {
                stored.insert(request.id.clone(), assignment.clone());
                changed = true;
            }
            bound.push(Entry {
                id: request.id.clone(),
                description: request.description.clone(),
                trigger: assignment.trigger,
                refused: None,
            });
        }
        let session = self.sessions.get_mut(handle).expect("session checked above");
        session.bound = true;
        // A later call adds to, or updates, what the session already holds.
        for entry in bound {
            match session.shortcuts.iter_mut().find(|held| held.id == entry.id) {
                Some(held) => *held = entry,
                None => session.shortcuts.push(entry),
            }
        }
        Ok(changed)
    }

    /// Every shortcut of the session with its current trigger, for the configuration panel.
    pub fn plan_configure(&self, handle: &str) -> Result<Vec<Pending>, RegistryError> {
        Ok(self
            .session(handle)?
            .shortcuts
            .iter()
            .map(|entry| Pending {
                id: entry.id.clone(),
                description: entry.description.clone(),
                preferred: None,
                current: entry.trigger.clone(),
            })
            .collect())
    }

    /// Apply the user's rebinding of a session's shortcuts. Returns whether stored choices changed.
    pub fn apply_choices(
        &mut self,
        handle: &str,
        choices: &[Choice],
    ) -> Result<bool, RegistryError> {
        let app_key = self.session(handle)?.app_key.clone();
        let stored = self.assignments.apps.entry(app_key).or_default();
        let session = self.sessions.get_mut(handle).expect("session checked above");
        let mut changed = false;
        for choice in choices {
            let Some(entry) = session.shortcuts.iter_mut().find(|e| e.id == choice.id) else {
                continue;
            };
            entry.trigger = choice.trigger.clone();
            entry.refused = None;
            let assignment = Assignment {
                trigger: choice.trigger.clone(),
                description: entry.description.clone(),
            };
            if stored.get(&choice.id) != Some(&assignment) {
                stored.insert(choice.id.clone(), assignment);
                changed = true;
            }
        }
        Ok(changed)
    }

    /// The session's shortcuts, or before the app has bound any, what it chose in earlier runs.
    pub fn list(&self, handle: &str) -> Result<Vec<View>, RegistryError> {
        let session = self.session(handle)?;
        if session.bound {
            return Ok(session
                .shortcuts
                .iter()
                .map(|entry| View {
                    id: entry.id.clone(),
                    description: entry.description.clone(),
                    trigger: entry
                        .refused
                        .is_none()
                        .then(|| entry.trigger.clone())
                        .flatten(),
                })
                .collect());
        }
        Ok(self
            .assignments
            .apps
            .get(&session.app_key)
            .into_iter()
            .flatten()
            .filter(|(_, assignment)| assignment.trigger.is_some())
            .map(|(id, assignment)| View {
                id: id.clone(),
                description: assignment.description.clone(),
                trigger: assignment.trigger.clone(),
            })
            .collect())
    }

    /// The keys to ask the compositor for, across all sessions. When two shortcuts want the same
    /// key the older session keeps it and the other is marked refused.
    pub fn plan_grabs(&mut self) -> Vec<Grab> {
        let mut taken: Vec<Trigger> = Vec::new();
        let mut grabs = Vec::new();
        let mut sessions: Vec<_> = self.sessions.values_mut().collect();
        sessions.sort_by_key(|session| session.serial);
        for session in sessions {
            for entry in &mut session.shortcuts {
                entry.refused = None;
                let Some(trigger) = &entry.trigger else {
                    continue;
                };
                if taken.contains(trigger) {
                    entry.refused = Some("already used by another shortcut".to_owned());
                    continue;
                }
                taken.push(trigger.clone());
                grabs.push(Grab {
                    id: format!("{}/{}", session.serial, entry.id),
                    key: trigger.to_niri(),
                });
            }
        }
        grabs
    }

    /// Record the compositor's refusals, by grab id.
    pub fn apply_refusals(&mut self, refused: &[(String, String)]) {
        for (grab_id, reason) in refused {
            let Some((serial, id)) = grab_id.split_once('/') else {
                continue;
            };
            let Ok(serial) = serial.parse::<u64>() else {
                continue;
            };
            let entry = self
                .sessions
                .values_mut()
                .find(|session| session.serial == serial)
                .and_then(|session| session.shortcuts.iter_mut().find(|e| e.id == id));
            if let Some(entry) = entry {
                entry.refused = Some(reason.clone());
            }
        }
    }

    /// The session and shortcut a grab id from the compositor refers to.
    pub fn resolve(&self, grab_id: &str) -> Option<(&str, &str)> {
        let (serial, id) = grab_id.split_once('/')?;
        let serial = serial.parse::<u64>().ok()?;
        let (handle, session) = self
            .sessions
            .iter()
            .find(|(_, session)| session.serial == serial)?;
        let entry = session.shortcuts.iter().find(|entry| entry.id == id)?;
        Some((handle.as_str(), entry.id.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger(text: &str) -> Trigger {
        Trigger::from_xdg(text).unwrap()
    }

    fn request(id: &str, preferred: Option<&str>) -> Requested {
        Requested {
            id: id.into(),
            description: format!("{id} description"),
            preferred_trigger: preferred.map(Into::into),
        }
    }

    fn choice(id: &str, text: Option<&str>) -> Choice {
        Choice {
            id: id.into(),
            trigger: text.map(trigger),
        }
    }

    fn registry_with(handle: &str, app: &str) -> Registry {
        let mut registry = Registry::default();
        registry.create_session(handle, app).unwrap();
        registry
    }

    #[test]
    fn unknown_and_duplicate_sessions_are_errors() {
        let mut registry = registry_with("/s/1", "app");
        assert_eq!(
            registry.create_session("/s/1", "app"),
            Err(RegistryError::DuplicateSession)
        );
        assert_eq!(
            registry.plan_bind("/s/2", &[]),
            Err(RegistryError::UnknownSession)
        );
        assert!(registry.close_session("/s/1"));
        assert!(!registry.close_session("/s/1"));
    }

    #[test]
    fn new_shortcuts_are_asked_about_with_their_preferred_trigger() {
        let registry = registry_with("/s/1", "app");
        let pending = registry
            .plan_bind(
                "/s/1",
                &[request("a", Some("CTRL+ALT+a")), request("b", Some("garbage+"))],
            )
            .unwrap();
        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].preferred, Some(trigger("CTRL+ALT+a")));
        assert_eq!(pending[1].preferred, None);
    }

    #[test]
    fn choices_persist_and_are_not_asked_again() {
        let mut registry = registry_with("/s/1", "app");
        let requested = [request("a", Some("CTRL+a")), request("b", None)];
        let changed = registry
            .commit_bind(
                "/s/1",
                &requested,
                &[choice("a", Some("CTRL+ALT+x")), choice("b", None)],
            )
            .unwrap();
        assert!(changed);

        // A new session of the same app gets the stored choices without a question.
        registry.create_session("/s/2", "app").unwrap();
        assert!(registry.plan_bind("/s/2", &requested).unwrap().is_empty());
        let changed = registry.commit_bind("/s/2", &requested, &[]).unwrap();
        assert!(!changed);
        let listed = registry.list("/s/2").unwrap();
        assert_eq!(listed[0].trigger, Some(trigger("CTRL+ALT+x")));
        assert_eq!(listed[1].trigger, None, "a declined shortcut stays declined");
    }

    #[test]
    fn list_before_binding_returns_earlier_choices() {
        let mut registry = registry_with("/s/1", "app");
        registry
            .commit_bind("/s/1", &[request("a", None)], &[choice("a", Some("F9"))])
            .unwrap();
        registry.create_session("/s/2", "app").unwrap();
        let listed = registry.list("/s/2").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].description, "a description");
        registry.create_session("/s/3", "other").unwrap();
        assert!(registry.list("/s/3").unwrap().is_empty());
    }

    #[test]
    fn a_later_bind_adds_to_the_session() {
        let mut registry = registry_with("/s/1", "app");
        registry
            .commit_bind("/s/1", &[request("a", None)], &[choice("a", Some("F1"))])
            .unwrap();
        registry
            .commit_bind("/s/1", &[request("b", None)], &[choice("b", Some("F2"))])
            .unwrap();
        let ids: Vec<_> = registry
            .list("/s/1")
            .unwrap()
            .into_iter()
            .map(|view| view.id)
            .collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn configuring_rebinds_and_persists() {
        let mut registry = registry_with("/s/1", "app");
        registry
            .commit_bind("/s/1", &[request("a", None)], &[choice("a", Some("F1"))])
            .unwrap();
        let plan = registry.plan_configure("/s/1").unwrap();
        assert_eq!(plan[0].current, Some(trigger("F1")));
        let changed = registry
            .apply_choices("/s/1", &[choice("a", Some("F2")), choice("ghost", Some("F3"))])
            .unwrap();
        assert!(changed);
        assert_eq!(registry.list("/s/1").unwrap()[0].trigger, Some(trigger("F2")));
        assert_eq!(
            registry.assignments().apps["app"]["a"].trigger,
            Some(trigger("F2"))
        );
        assert!(!registry.assignments().apps["app"].contains_key("ghost"));
    }

    #[test]
    fn the_older_session_keeps_a_contested_trigger() {
        let mut registry = registry_with("/s/1", "one");
        registry.create_session("/s/2", "two").unwrap();
        registry
            .commit_bind("/s/2", &[request("y", None)], &[choice("y", Some("ctrl+k"))])
            .unwrap();
        registry
            .commit_bind("/s/1", &[request("x", None)], &[choice("x", Some("CTRL+K"))])
            .unwrap();
        let grabs = registry.plan_grabs();
        assert_eq!(grabs.len(), 1);
        assert_eq!(grabs[0].key, "Ctrl+K");
        assert_eq!(registry.resolve(&grabs[0].id), Some(("/s/1", "x")));
        assert_eq!(registry.list("/s/2").unwrap()[0].trigger, None);
    }

    #[test]
    fn compositor_refusals_hide_the_trigger_until_the_next_plan() {
        let mut registry = registry_with("/s/1", "app");
        registry
            .commit_bind("/s/1", &[request("a", None)], &[choice("a", Some("F1"))])
            .unwrap();
        let grabs = registry.plan_grabs();
        registry.apply_refusals(&[(grabs[0].id.clone(), "already a configured bind".into())]);
        assert_eq!(registry.list("/s/1").unwrap()[0].trigger, None);
        // The choice is kept, and retried once the conflict is gone.
        assert_eq!(registry.plan_grabs().len(), 1);
        assert_eq!(registry.list("/s/1").unwrap()[0].trigger, Some(trigger("F1")));
    }

    #[test]
    fn resolve_rejects_stale_and_malformed_ids() {
        let mut registry = registry_with("/s/1", "app");
        registry
            .commit_bind("/s/1", &[request("a/b", None)], &[choice("a/b", Some("F1"))])
            .unwrap();
        let grabs = registry.plan_grabs();
        assert_eq!(registry.resolve(&grabs[0].id), Some(("/s/1", "a/b")));
        registry.close_session("/s/1");
        assert_eq!(registry.resolve(&grabs[0].id), None);
        assert_eq!(registry.resolve("nonsense"), None);
        assert_eq!(registry.resolve("x/y"), None);
    }

    #[test]
    fn apps_without_an_id_share_a_bucket() {
        let mut registry = registry_with("/s/1", "");
        registry
            .commit_bind("/s/1", &[request("a", None)], &[choice("a", Some("F1"))])
            .unwrap();
        assert_eq!(registry.app_id("/s/1").unwrap(), "");
        assert!(registry.assignments().apps.contains_key(UNREGISTERED));
    }
}
