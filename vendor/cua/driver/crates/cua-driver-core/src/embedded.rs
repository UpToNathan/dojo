//! Read-only ownership feasibility experiment, not a desktop runtime.
//!
//! The imported engine is deliberately unreachable: its transport mutation and
//! lifecycle context have not passed the external admission proof.
//!
//! Private imports are a compile error for consumers.
//! ```compile_fail
//! use cua_driver_core::browser::BrowserEngine;
//! ```
//! ```compile_fail
//! use cua_driver_core::session::revive_session;
//! ```
//! ```compile_fail
//! use cua_driver_core::tool::ToolRegistry;
//! ```
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::element_token::{token_for, ResolvedElement};
use crate::snapshot_store::{SnapshotPayload, SnapshotStore};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ExactWindow {
    pub compositor_epoch: Uuid,
    pub window_id: u64,
    pub client_generation: u64,
    pub pid: i32,
    pub process_start: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementRef {
    owner: Uuid,
    target: ExactWindow,
    token: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    Closed,
    Cancelled,
    InvalidElementIndices,
    WrongEpisode,
    Stale,
    IdentityChanged,
    MutationAdmissionUnproved,
}

struct Published {
    target: ExactWindow,
    snapshot: u32,
}
struct State<S: SnapshotPayload> {
    open: bool,
    observation_epoch: Uuid,
    snapshots: SnapshotStore<S>,
    published: HashMap<u64, Published>,
}

/// Created by the service for an authenticated connection. No caller session
/// label is accepted. The UUID is both a runtime epoch and an episode key.
///
/// ```
/// use cua_driver_core::embedded::{EmbeddedEpisode, RetainedElements, Refusal};
/// let episode = EmbeddedEpisode::<RetainedElements>::new();
/// assert_eq!(episode.request_mutation(), Err(Refusal::MutationAdmissionUnproved));
/// episode.close();
/// assert_eq!(episode.request_mutation(), Err(Refusal::Closed));
/// ```
pub struct EmbeddedEpisode<S: SnapshotPayload> {
    key: Uuid,
    state: Arc<Mutex<State<S>>>,
}

/// A read started before shutdown can hold this ticket. Publication rechecks
/// liveness under the same mutex as shutdown; teardown therefore wins reliably.
pub struct Publication<S: SnapshotPayload> {
    key: Uuid,
    observation_epoch: Uuid,
    state: Arc<Mutex<State<S>>>,
    target: ExactWindow,
}

impl<S: SnapshotPayload> EmbeddedEpisode<S> {
    pub fn new() -> Self {
        let key = Uuid::new_v4();
        Self {
            key,
            state: Arc::new(Mutex::new(State {
                open: true,
                observation_epoch: Uuid::new_v4(),
                snapshots: SnapshotStore::new_for_runtime_scope(key.to_string()),
                published: HashMap::new(),
            })),
        }
    }

    pub fn begin_observation(&self, target: ExactWindow) -> Result<Publication<S>, Refusal> {
        let state = self.state.lock().unwrap();
        if !state.open {
            return Err(Refusal::Closed);
        }
        Ok(Publication {
            key: self.key,
            observation_epoch: state.observation_epoch,
            state: Arc::clone(&self.state),
            target,
        })
    }

    pub fn resolve(
        &self,
        reference: &ElementRef,
        live: ExactWindow,
    ) -> Result<S::Element, Refusal> {
        let state = self.state.lock().unwrap();
        if !state.open {
            return Err(Refusal::Closed);
        }
        if reference.owner != self.key {
            return Err(Refusal::WrongEpisode);
        }
        if reference.target != live {
            return Err(Refusal::IdentityChanged);
        }
        let observed = state.published.get(&live.window_id).ok_or(Refusal::Stale)?;
        if observed.target != live {
            return Err(Refusal::Stale);
        }
        match state.snapshots.resolve(
            live.pid,
            &serde_json::json!({"element_token": reference.token, "window_id": live.window_id}),
        ) {
            Ok(ResolvedElement::Element { element, .. }) => Ok(element),
            _ => Err(Refusal::Stale),
        }
    }

    pub fn request_mutation(&self) -> Result<(), Refusal> {
        let state = self.state.lock().unwrap();
        if !state.open {
            return Err(Refusal::Closed);
        }
        Err(Refusal::MutationAdmissionUnproved)
    }

    pub fn cancel_pending_observations(&self) {
        let mut state = self.state.lock().unwrap();
        state.observation_epoch = Uuid::new_v4();
    }

    pub fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.open = false;
        state.snapshots.clear();
        state.published.clear();
    }
}
impl<S: SnapshotPayload> Default for EmbeddedEpisode<S> {
    fn default() -> Self {
        Self::new()
    }
}
impl<S: SnapshotPayload> Drop for EmbeddedEpisode<S> {
    fn drop(&mut self) {
        self.close();
    }
}

impl<S: SnapshotPayload> Publication<S> {
    pub fn publish(self, payload: S, indices: &[usize]) -> Result<Vec<ElementRef>, Refusal> {
        let mut state = self.state.lock().unwrap();
        if !state.open {
            return Err(Refusal::Closed);
        }
        if state.observation_epoch != self.observation_epoch {
            return Err(Refusal::Cancelled);
        }
        if indices.len() > payload.len()
            || indices.iter().copied().collect::<HashSet<_>>().len() != indices.len()
            || indices.iter().any(|index| payload.retain(*index).is_none())
        {
            return Err(Refusal::InvalidElementIndices);
        }
        if let Some(previous) = state.published.remove(&self.target.window_id) {
            state
                .snapshots
                .remove(previous.target.pid, self.target.window_id);
        }
        // This episode owns an unregistered, explicitly scoped private store.
        // No CUA global session lifecycle decides whether it is open.
        let (snapshot, retired) = state
            .snapshots
            .publish_for_session(self.target.pid, self.target.window_id, payload, None, None)
            .ok_or(Refusal::Closed)?;
        state
            .published
            .retain(|_, entry| !retired.contains(&entry.snapshot));
        state.published.insert(
            self.target.window_id,
            Published {
                target: self.target,
                snapshot,
            },
        );
        Ok(indices
            .iter()
            .copied()
            .map(|index| ElementRef {
                owner: self.key,
                target: self.target,
                token: token_for(snapshot, index),
            })
            .collect())
    }
}

/// Selective read-only snapshot payload retained behind the imported store.
/// Real AT-SPI payloads can implement the same private trait in this crate.
#[derive(Clone)]
pub struct RetainedElements(pub Vec<String>);
impl SnapshotPayload for RetainedElements {
    type Element = String;
    fn len(&self) -> usize {
        self.0.len()
    }
    fn retain(&self, index: usize) -> Option<String> {
        self.0.get(index).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target() -> ExactWindow {
        ExactWindow {
            compositor_epoch: Uuid::new_v4(),
            window_id: 7,
            client_generation: 1,
            pid: 42,
            process_start: 100,
        }
    }
    fn publish(episode: &EmbeddedEpisode<RetainedElements>, target: ExactWindow) -> ElementRef {
        episode
            .begin_observation(target)
            .unwrap()
            .publish(RetainedElements(vec!["submit".into()]), &[0])
            .unwrap()
            .remove(0)
    }
    #[test]
    fn sparse_original_indices_are_retained_and_never_renumbered() {
        struct Sparse;
        impl SnapshotPayload for Sparse {
            type Element = &'static str;
            fn len(&self) -> usize {
                1
            }
            fn retain(&self, index: usize) -> Option<Self::Element> {
                (index == 17).then_some("original index 17")
            }
        }
        let episode = EmbeddedEpisode::<Sparse>::new();
        let target = target();
        let refs = episode
            .begin_observation(target)
            .unwrap()
            .publish(Sparse, &[17])
            .unwrap();
        assert_eq!(episode.resolve(&refs[0], target), Ok("original index 17"));
        assert_eq!(
            episode
                .begin_observation(target)
                .unwrap()
                .publish(Sparse, &[0]),
            Err(Refusal::InvalidElementIndices)
        );
    }
    #[test]
    fn retained_object_resolves_without_rewalking() {
        let episode = EmbeddedEpisode::new();
        let target = target();
        assert_eq!(
            episode.resolve(&publish(&episode, target), target),
            Ok("submit".into())
        );
    }
    #[test]
    fn another_episode_cannot_resolve_a_reference() {
        let first = EmbeddedEpisode::new();
        let second = EmbeddedEpisode::<RetainedElements>::new();
        let target = target();
        let reference = publish(&first, target);
        publish(&second, target);
        assert_eq!(
            second.resolve(&reference, target),
            Err(Refusal::WrongEpisode)
        );
    }
    #[test]
    fn closed_episode_refuses_reads_publication_and_mutations() {
        let episode = EmbeddedEpisode::new();
        let target = target();
        let reference = publish(&episode, target);
        let pending = episode.begin_observation(target).unwrap();
        episode.close();
        episode.close();
        assert_eq!(episode.resolve(&reference, target), Err(Refusal::Closed));
        assert_eq!(
            pending.publish(RetainedElements(vec!["late".into()]), &[0]),
            Err(Refusal::Closed)
        );
        assert!(matches!(
            episode.begin_observation(target),
            Err(Refusal::Closed)
        ));
        assert_eq!(episode.request_mutation(), Err(Refusal::Closed));
    }
    #[test]
    fn changed_process_or_compositor_or_client_generation_refuses() {
        let episode = EmbeddedEpisode::new();
        let original = target();
        let reference = publish(&episode, original);
        for changed in [
            ExactWindow {
                process_start: 101,
                ..original
            },
            ExactWindow {
                compositor_epoch: Uuid::new_v4(),
                ..original
            },
            ExactWindow {
                client_generation: 2,
                ..original
            },
        ] {
            assert_eq!(
                episode.resolve(&reference, changed),
                Err(Refusal::IdentityChanged)
            );
        }
    }
    #[test]
    fn replacement_invalidates_previous_observation() {
        let episode = EmbeddedEpisode::new();
        let target = target();
        let old = publish(&episode, target);
        let current = publish(&episode, target);
        assert_eq!(episode.resolve(&old, target), Err(Refusal::Stale));
        assert_eq!(episode.resolve(&current, target), Ok("submit".into()));
    }
    #[test]
    fn cancellation_rejects_pending_work_and_allows_a_new_read() {
        let episode = EmbeddedEpisode::<RetainedElements>::new();
        let target = target();
        let pending = episode.begin_observation(target).unwrap();
        episode.cancel_pending_observations();
        assert_eq!(
            pending.publish(RetainedElements(vec!["late".into()]), &[0]),
            Err(Refusal::Cancelled)
        );
        assert_eq!(
            episode.resolve(&publish(&episode, target), target),
            Ok("submit".into())
        );
    }
    #[test]
    fn dropping_owner_closes_pending_publication() {
        let episode = EmbeddedEpisode::<RetainedElements>::new();
        let pending = episode.begin_observation(target()).unwrap();
        drop(episode);
        assert_eq!(
            pending.publish(RetainedElements(vec!["late".into()]), &[0]),
            Err(Refusal::Closed)
        );
    }
    #[test]
    fn publication_racing_shutdown_cannot_survive_shutdown() {
        let episode = EmbeddedEpisode::<RetainedElements>::new();
        let target = target();
        let pending = episode.begin_observation(target).unwrap();
        let worker = std::thread::spawn(move || {
            pending.publish(RetainedElements(vec!["submit".into()]), &[0])
        });
        episode.close();
        if let Ok(references) = worker.join().unwrap() {
            assert_eq!(
                episode.resolve(&references[0], target),
                Err(Refusal::Closed)
            );
        }
    }
    #[test]
    fn mutation_is_refused_before_a_backend_exists() {
        let episode = EmbeddedEpisode::<RetainedElements>::new();
        assert_eq!(
            episode.request_mutation(),
            Err(Refusal::MutationAdmissionUnproved)
        );
    }
    #[test]
    fn upstream_semantic_resolver_ignores_screenshot_owner() {
        let store = SnapshotStore::new();
        let (snapshot, _) = store
            .publish_for_session(
                42,
                7,
                RetainedElements(vec!["submit".into()]),
                Some("owner-a"),
                None,
            )
            .unwrap();
        assert!(matches!(
            store.resolve(
                42,
                &serde_json::json!({
                    "element_token": token_for(snapshot, 0), "window_id": 7
                })
            ),
            Ok(ResolvedElement::Element { .. })
        ));
        // resolve has no session argument: possession bypasses owner binding.
    }
    #[test]
    fn upstream_lifecycle_can_revive_after_end_and_anonymous_publication_bypasses_end() {
        let owner = format!("spike-ended-{}", Uuid::new_v4());
        let store = SnapshotStore::new();
        crate::session::end_session(&owner);
        assert!(crate::session::is_session_ended(&owner));
        assert!(store
            .publish_for_session(
                42,
                7,
                RetainedElements(vec!["late".into()]),
                Some(&owner),
                None
            )
            .is_none());
        let snapshot = store.publish(42, 7, RetainedElements(vec!["anonymous".into()]));
        assert!(matches!(
            store.resolve(
                42,
                &serde_json::json!({
                    "element_token": token_for(snapshot, 0), "window_id": 7
                })
            ),
            Ok(ResolvedElement::Element { .. })
        ));
        assert!(crate::session::revive_session(&owner));
        assert!(!crate::session::is_session_ended(&owner));
        crate::session::end_session(&owner);
    }
    #[test]
    fn upstream_bare_rows_resolve_against_new_current_snapshot() {
        let store = SnapshotStore::new();
        store.publish(42, 7, RetainedElements(vec!["new".into()]));
        assert!(
            matches!(store.resolve(42, &serde_json::json!({"element_token": "0", "window_id": 7})),
            Ok(ResolvedElement::Element { element, .. }) if element == "new")
        );
    }
}
