//! Consumer for the pinned xdg-dbus-annotation protocol.
//!
//! Only AT-SPI Accessible annotations on the AT-SPI bus become accessibility
//! endpoints. Values are staged by protocol requests and become visible at
//! wl_surface.commit.
use super::raw::xdg_dbus_annotation::server::{
    xdg_dbus_annotation_manager_v1::{self, XdgDbusAnnotationManagerV1},
    xdg_dbus_annotation_v1::{self, BusEnum, XdgDbusAnnotationV1},
};
use crate::niri::State;
use niri_ipc::AccessibilityEndpoint;
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, DisplayHandle, New, Resource, WEnum,
};
use smithay::wayland::{compositor::with_states, Dispatch2, GlobalDispatch2};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

const ACCESSIBLE_INTERFACE: &str = "org.a11y.atspi.Accessible";
const AT_SPI_BUS: u32 = 2;
static GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Values {
    bus: Option<u32>,
    bus_name: Option<String>,
    object_path: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum EndpointState {
    Unset,
    Bound(AccessibilityEndpoint),
    Invalid,
}

struct Entry<K> {
    resource: K,
    interface: String,
    values: Values,
    pending: Values,
    active: bool,
    committed_once: bool,
    endpoint: EndpointState,
}

struct Annotation<K> {
    entries: Vec<Entry<K>>,
    generation: u64,
}

impl<K> Default for Annotation<K> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            generation: 0,
        }
    }
}

impl<K: PartialEq> Annotation<K> {
    fn changed(&mut self) {
        self.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    }

    fn attach(&mut self, resource: K, interface: String) -> bool {
        if self
            .entries
            .iter()
            .any(|entry| entry.interface == interface)
        {
            return false;
        }
        self.entries.push(Entry {
            resource,
            interface,
            values: Values::default(),
            pending: Values::default(),
            active: false,
            committed_once: false,
            endpoint: EndpointState::Unset,
        });
        self.changed();
        true
    }

    fn remove(&mut self, resource: &K) {
        let before = self.entries.len();
        self.entries.retain(|entry| &entry.resource != resource);
        if before != self.entries.len() {
            self.changed();
        }
    }

    fn stage_bus(&mut self, resource: &K, bus: u32) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.resource == resource)
        {
            entry.pending.bus = Some(bus);
        }
    }

    fn stage_name(&mut self, resource: &K, name: String) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.resource == resource)
        {
            entry.pending.bus_name = Some(name);
        }
    }

    fn stage_path(&mut self, resource: &K, path: String) {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| &entry.resource == resource)
        {
            entry.pending.object_path = Some(path);
        }
    }

    /// Apply staged state at the surface commit boundary.
    fn commit(&mut self) -> Vec<K>
    where
        K: Clone,
    {
        let mut incomplete = Vec::new();
        let mut changed = false;
        for entry in &mut self.entries {
            let first_commit = !entry.committed_once;
            entry.committed_once = true;
            if entry.pending != Values::default() {
                if let Some(bus) = entry.pending.bus.take() {
                    entry.values.bus = Some(bus);
                }
                if let Some(name) = entry.pending.bus_name.take() {
                    entry.values.bus_name = Some(name);
                }
                if let Some(path) = entry.pending.object_path.take() {
                    entry.values.object_path = Some(path);
                }
                entry.active = true;
                entry.endpoint = classify(&entry.interface, &entry.values);
                changed = true;
            }
            if first_commit {
                entry.active = true;
                changed = true;
            }
            if entry.active
                && (entry.values.bus.is_none()
                    || entry.values.bus_name.is_none()
                    || entry.values.object_path.is_none())
            {
                incomplete.push(entry.resource.clone());
                entry.endpoint = EndpointState::Invalid;
            }
        }
        if changed {
            self.changed();
        }
        incomplete
    }

    fn retire(&mut self) {
        let mut changed = false;
        for entry in &mut self.entries {
            if entry.active {
                entry.active = false;
                entry.endpoint = EndpointState::Unset;
                changed = true;
            }
        }
        if changed {
            self.changed();
        }
    }

    fn read(&self) -> Result<(u64, Option<AccessibilityEndpoint>), String> {
        let relevant: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.interface == ACCESSIBLE_INTERFACE && entry.active)
            .collect();
        if relevant.len() > 1 {
            return Err(
                "multiple generic accessibility annotations make this surface ambiguous".into(),
            );
        }
        match relevant.first().map(|entry| &entry.endpoint) {
            None | Some(EndpointState::Unset) => Ok((self.generation, None)),
            Some(EndpointState::Bound(endpoint)) => Ok((self.generation, Some(endpoint.clone()))),
            Some(EndpointState::Invalid) => {
                Err("native surface announced an invalid accessibility endpoint".into())
            }
        }
    }
}

fn classify(interface: &str, values: &Values) -> EndpointState {
    if interface != ACCESSIBLE_INTERFACE {
        return EndpointState::Unset;
    }
    let (Some(bus), Some(bus_name), Some(object_path)) = (
        values.bus,
        values.bus_name.as_ref(),
        values.object_path.as_ref(),
    ) else {
        return EndpointState::Invalid;
    };
    if bus != AT_SPI_BUS {
        return EndpointState::Invalid;
    }
    valid_endpoint(bus_name, object_path).map_or(EndpointState::Invalid, EndpointState::Bound)
}

#[derive(Default)]
struct SurfaceState(Mutex<Annotation<XdgDbusAnnotationV1>>);

pub struct ManagerData;
pub struct AnnotationData {
    surface: WlSurface,
}

pub fn register(display: &DisplayHandle) {
    display.create_global::<State, XdgDbusAnnotationManagerV1, _>(1, ManagerData);
}

/// Returns this protocol's own generation and active endpoint. The caller
/// combines this with GTK's independent source and rejects disagreement.
pub fn endpoint(surface: &WlSurface) -> Result<(u64, Option<AccessibilityEndpoint>), String> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<SurfaceState>()
            .map_or(Ok((0, None)), |data| data.0.lock().unwrap().read())
    })
}

/// Publish staged annotation values after the corresponding surface commit.
pub fn commit(surface: &WlSurface) {
    let incomplete = with_states(surface, |states| {
        states
            .data_map
            .get::<SurfaceState>()
            .map(|data| data.0.lock().unwrap().commit())
            .unwrap_or_default()
    });
    for annotation in incomplete {
        annotation.post_error(
            xdg_dbus_annotation_v1::Error::Incomplete,
            "D-Bus annotation was committed before all required fields were set",
        );
    }
}

/// Retire active endpoints when the native surface is unmapped.
pub fn retire(surface: &WlSurface) {
    with_states(surface, |states| {
        if let Some(data) = states.data_map.get::<SurfaceState>() {
            data.0.lock().unwrap().retire();
        }
    });
}

impl GlobalDispatch2<XdgDbusAnnotationManagerV1, State> for ManagerData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<XdgDbusAnnotationManagerV1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        data_init.init(resource, ManagerData);
    }
}

impl Dispatch2<XdgDbusAnnotationManagerV1, State> for ManagerData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &XdgDbusAnnotationManagerV1,
        request: xdg_dbus_annotation_manager_v1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            xdg_dbus_annotation_manager_v1::Request::Destroy => {}
            xdg_dbus_annotation_manager_v1::Request::AnnotateSurface {
                interface,
                id,
                surface,
            } => {
                let annotation = data_init.init(
                    id,
                    AnnotationData {
                        surface: surface.clone(),
                    },
                );
                let created = with_states(&surface, |states| {
                    states.data_map.insert_if_missing(SurfaceState::default);
                    states
                        .data_map
                        .get::<SurfaceState>()
                        .unwrap()
                        .0
                        .lock()
                        .unwrap()
                        .attach(annotation.clone(), interface)
                });
                if !created {
                    resource.post_error(
                        xdg_dbus_annotation_manager_v1::Error::AlreadyAnnotated,
                        "surface already has an annotation for this interface",
                    );
                }
            }
        }
    }
}

impl Dispatch2<XdgDbusAnnotationV1, State> for AnnotationData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        resource: &XdgDbusAnnotationV1,
        request: xdg_dbus_annotation_v1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        with_states(&self.surface, |states| {
            let Some(data) = states.data_map.get::<SurfaceState>() else {
                return;
            };
            let mut annotation = data.0.lock().unwrap();
            match request {
                xdg_dbus_annotation_v1::Request::Destroy => annotation.remove(resource),
                xdg_dbus_annotation_v1::Request::SetBus { bus } => match bus {
                    WEnum::Value(BusEnum::Atspi) => annotation.stage_bus(resource, AT_SPI_BUS),
                    WEnum::Value(BusEnum::Session) => annotation.stage_bus(resource, 0),
                    WEnum::Value(BusEnum::System) => annotation.stage_bus(resource, 1),
                    WEnum::Unknown(value) => annotation.stage_bus(resource, value),
                },
                xdg_dbus_annotation_v1::Request::SetBusName { bus_name } => {
                    annotation.stage_name(resource, bus_name)
                }
                xdg_dbus_annotation_v1::Request::SetObjectPath { object_path } => {
                    annotation.stage_path(resource, object_path)
                }
            }
        });
    }

    fn destroyed(
        &self,
        _state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &XdgDbusAnnotationV1,
    ) {
        with_states(&self.surface, |states| {
            if let Some(data) = states.data_map.get::<SurfaceState>() {
                data.0.lock().unwrap().remove(resource);
            }
        });
    }
}

fn valid_endpoint(bus_name: &str, object_path: &str) -> Option<AccessibilityEndpoint> {
    let valid_name = bus_name.len() <= 255
        && bus_name.starts_with(':')
        && bus_name[1..].contains('.')
        && bus_name[1..].split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
        });
    let valid_path = object_path.len() <= 4096
        && (object_path == "/"
            || object_path.starts_with('/')
                && object_path[1..].split('/').all(|part| {
                    !part.is_empty() && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
                }));
    (valid_name && valid_path).then(|| AccessibilityEndpoint {
        bus_name: bus_name.to_owned(),
        object_path: object_path.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn announce(annotation: &mut Annotation<u64>, owner: u64, bus: u32, name: &str, path: &str) {
        annotation.stage_bus(&owner, bus);
        annotation.stage_name(&owner, name.to_owned());
        annotation.stage_path(&owner, path.to_owned());
    }

    fn one_accessible() -> Annotation<u64> {
        let mut annotation = Annotation::default();
        assert!(annotation.attach(1, ACCESSIBLE_INTERFACE.into()));
        annotation
    }

    #[test]
    fn annotation_is_not_visible_until_surface_commit() {
        let mut annotation = one_accessible();
        announce(&mut annotation, 1, AT_SPI_BUS, ":1.42", "/org/a11y/app");
        assert_eq!(annotation.read().unwrap().1, None);
        annotation.commit();
        assert_eq!(
            annotation.read().unwrap().1,
            Some(AccessibilityEndpoint {
                bus_name: ":1.42".into(),
                object_path: "/org/a11y/app".into(),
            })
        );
    }

    #[test]
    fn replacement_and_retirement_change_generation_and_retire_root() {
        let mut annotation = one_accessible();
        announce(&mut annotation, 1, AT_SPI_BUS, ":1.42", "/first");
        annotation.commit();
        let first = annotation.read().unwrap().0;
        announce(&mut annotation, 1, AT_SPI_BUS, ":1.42", "/second");
        assert_eq!(annotation.read().unwrap().1.unwrap().object_path, "/first");
        annotation.commit();
        let second = annotation.read().unwrap().0;
        assert_ne!(first, second);
        assert_eq!(annotation.read().unwrap().1.unwrap().object_path, "/second");
        annotation.retire();
        let (retired, endpoint) = annotation.read().unwrap();
        assert_ne!(second, retired);
        assert!(endpoint.is_none());
    }

    #[test]
    fn incomplete_or_wrong_bus_accessibility_annotation_fails_closed() {
        let mut incomplete = one_accessible();
        incomplete.stage_bus(&1, AT_SPI_BUS);
        incomplete.commit();
        assert_eq!(
            incomplete.read().unwrap_err(),
            "native surface announced an invalid accessibility endpoint"
        );

        let mut wrong_bus = one_accessible();
        announce(&mut wrong_bus, 1, 1, ":1.42", "/root");
        wrong_bus.commit();
        assert_eq!(
            wrong_bus.read().unwrap_err(),
            "native surface announced an invalid accessibility endpoint"
        );
    }

    #[test]
    fn duplicate_interface_is_refused_but_other_interfaces_are_independent() {
        let mut annotation = one_accessible();
        assert!(!annotation.attach(2, ACCESSIBLE_INTERFACE.into()));
        assert!(annotation.attach(3, "org.example.Menu".into()));
    }

    #[test]
    fn only_unique_names_and_valid_bounded_paths_are_accepted() {
        assert!(valid_endpoint(":1.32", "/org/a11y/window_1").is_some());
        for name in ["org.example.App", ":1", ":.1", ":1..3", ":1/2.3"] {
            assert!(valid_endpoint(name, "/window").is_none());
        }
        for path in ["", "window", "/window/", "/window//child", "/bad-path"] {
            assert!(valid_endpoint(":1.2", path).is_none());
        }
    }
}
