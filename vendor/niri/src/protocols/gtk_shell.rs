//! GTK's deployed surface-to-AT-SPI association. No semantic input is admitted here.
use super::raw::gtk_shell::server::{
    gtk_shell1::{self, GtkShell1},
    gtk_surface1::{self, GtkSurface1},
};
use crate::niri::State;
use niri_ipc::AccessibilityEndpoint;
use smithay::reexports::wayland_server::{
    protocol::wl_surface::WlSurface, Client, DataInit, DisplayHandle, New, Resource, WEnum,
};
use smithay::utils::Serial;
use smithay::wayland::{
    compositor::with_states,
    shell::xdg::ToplevelSurface,
    xdg_activation::{XdgActivationHandler, XdgActivationToken},
    Dispatch2, GlobalDispatch2,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Mutex,
};

static GENERATION: AtomicU64 = AtomicU64::new(1);

enum EndpointState {
    Unset,
    Bound(AccessibilityEndpoint),
    Invalid,
}

struct Annotation<K> {
    wrappers: Vec<(K, EndpointState)>,
    generation: u64,
}
impl<K> Default for Annotation<K> {
    fn default() -> Self {
        Self {
            wrappers: Vec::new(),
            generation: 0,
        }
    }
}
impl<K: PartialEq> Annotation<K> {
    fn changed(&mut self) {
        self.generation = GENERATION.fetch_add(1, Ordering::Relaxed);
    }
    fn attach(&mut self, owner: K) {
        if self.wrappers.iter().any(|(key, _)| key == &owner) {
            return;
        }
        self.wrappers.push((owner, EndpointState::Unset));
        self.changed();
    }
    fn announce(&mut self, owner: &K, bus_name: String, object_path: String) {
        if let Some((_, endpoint)) = self.wrappers.iter_mut().find(|(key, _)| key == owner) {
            *endpoint = valid_endpoint(bus_name, object_path)
                .map_or(EndpointState::Invalid, EndpointState::Bound);
            self.changed();
        }
    }
    fn remove(&mut self, owner: &K) {
        let before = self.wrappers.len();
        self.wrappers.retain(|(key, _)| key != owner);
        if self.wrappers.len() != before {
            self.changed();
        }
    }
    fn retire(&mut self) {
        for (_, endpoint) in &mut self.wrappers {
            *endpoint = EndpointState::Unset;
        }
        self.changed();
    }
    fn read(&self) -> Result<(u64, Option<AccessibilityEndpoint>), String> {
        if self.wrappers.len() > 1 {
            return Err(
                "multiple GTK wrappers make this surface's accessibility binding ambiguous".into(),
            );
        }
        match self.wrappers.first().map(|(_, endpoint)| endpoint) {
            None | Some(EndpointState::Unset) => Ok((self.generation, None)),
            Some(EndpointState::Bound(endpoint)) => Ok((self.generation, Some(endpoint.clone()))),
            Some(EndpointState::Invalid) => {
                Err("native surface announced an invalid accessibility endpoint".into())
            }
        }
    }
}
#[derive(Default)]
struct SurfaceState(Mutex<Annotation<GtkSurface1>>);
pub struct ManagerData;
pub struct SurfaceData {
    surface: WlSurface,
}

pub fn register(display: &DisplayHandle) {
    display.create_global::<State, GtkShell1, _>(7, ManagerData);
}

pub fn endpoint(surface: &WlSurface) -> Result<(u64, Option<AccessibilityEndpoint>), String> {
    with_states(surface, |states| {
        states
            .data_map
            .get::<SurfaceState>()
            .map_or(Ok((0, None)), |data| data.0.lock().unwrap().read())
    })
}

pub fn retire(surface: &WlSurface) {
    with_states(surface, |states| {
        if let Some(data) = states.data_map.get::<SurfaceState>() {
            data.0.lock().unwrap().retire();
        }
    });
}

/// Send GTK hints immediately before the corresponding xdg configure.
pub fn configure(toplevel: &ToplevelSurface) {
    let wrappers = with_states(toplevel.wl_surface(), |states| {
        states
            .data_map
            .get::<SurfaceState>()
            .map(|data| {
                data.0
                    .lock()
                    .unwrap()
                    .wrappers
                    .iter()
                    .map(|(wrapper, _)| wrapper.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });

    if wrappers.is_empty() {
        return;
    }

    use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::State as XdgState;
    let (tiled, resizable) = toplevel.with_pending_state(|pending| {
        (
            pending.states.contains(XdgState::TiledLeft),
            !pending.states.contains(XdgState::Fullscreen)
                && !pending.states.contains(XdgState::Maximized),
        )
    });
    for wrapper in wrappers {
        wrapper.configure(tiled_states(tiled, wrapper.version()));
        if wrapper.version() >= 2 {
            let edges: Vec<u8> = if resizable {
                vec![1u32, 2, 3, 4]
            } else {
                vec![]
            }
            .into_iter()
            .flat_map(u32::to_ne_bytes)
            .collect();
            wrapper.configure_edges(edges);
        }
    }
}

fn tiled_states(tiled: bool, version: u32) -> Vec<u8> {
    if tiled && version >= 2 {
        vec![1u32, 2, 3, 4, 5]
    } else if tiled {
        vec![1u32]
    } else {
        vec![]
    }
    .into_iter()
    .flat_map(u32::to_ne_bytes)
    .collect()
}

impl GlobalDispatch2<GtkShell1, State> for ManagerData {
    fn bind(
        &self,
        _state: &mut State,
        _handle: &DisplayHandle,
        _client: &Client,
        resource: New<GtkShell1>,
        data_init: &mut DataInit<'_, State>,
    ) {
        let resource = data_init.init(resource, ManagerData);
        resource.capabilities(0);
    }
}
impl Dispatch2<GtkShell1, State> for ManagerData {
    fn request(
        &self,
        _state: &mut State,
        _client: &Client,
        _resource: &GtkShell1,
        request: gtk_shell1::Request,
        _handle: &DisplayHandle,
        data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            gtk_shell1::Request::GetGtkSurface {
                gtk_surface,
                surface,
            } => {
                let resource = data_init.init(
                    gtk_surface,
                    SurfaceData {
                        surface: surface.clone(),
                    },
                );
                with_states(&surface, |states| {
                    states.data_map.insert_if_missing(SurfaceState::default);
                    let mut annotation = states
                        .data_map
                        .get::<SurfaceState>()
                        .unwrap()
                        .0
                        .lock()
                        .unwrap();
                    annotation.attach(resource);
                });
            }
            // GTK startup notifications and the optional audible bell carry no
            // activation authority. Existing xdg-activation remains authoritative.
            gtk_shell1::Request::SetStartupId { .. }
            | gtk_shell1::Request::NotifyLaunch { .. }
            | gtk_shell1::Request::SystemBell { .. } => {}
        }
    }
}
impl Dispatch2<GtkSurface1, State> for SurfaceData {
    fn request(
        &self,
        state: &mut State,
        _client: &Client,
        resource: &GtkSurface1,
        request: gtk_surface1::Request,
        _handle: &DisplayHandle,
        _data_init: &mut DataInit<'_, State>,
    ) {
        match request {
            gtk_surface1::Request::SetA11yProperties {
                a11y_dbus_name,
                toplevel_object_path,
            } => {
                with_states(&self.surface, |states| {
                    let mut annotation = states
                        .data_map
                        .get::<SurfaceState>()
                        .unwrap()
                        .0
                        .lock()
                        .unwrap();
                    annotation.announce(resource, a11y_dbus_name, toplevel_object_path);
                });
            }
            gtk_surface1::Request::Release => self.clear(resource),
            gtk_surface1::Request::RequestFocus { startup_id } => {
                let Some(startup_id) = startup_id else { return };
                let token = XdgActivationToken::from(startup_id);
                if let Some(data) = state.niri.activation_state.data_for_token(&token).cloned() {
                    state.request_activation(token, data, self.surface.clone());
                }
            }
            gtk_surface1::Request::TitlebarGesture {
                serial,
                seat,
                gesture,
            } => {
                let pointer = state.niri.seat.get_pointer().unwrap();
                let valid = state.niri.seat.owns(&seat)
                    && pointer.has_grab(Serial::from(serial))
                    && pointer
                        .current_focus()
                        .is_some_and(|surface| surface == self.surface);
                if valid && gesture == WEnum::Value(gtk_surface1::Gesture::DoubleClick) {
                    if let Some((mapped, _)) =
                        state.niri.layout.find_window_and_output(&self.surface)
                    {
                        let window = mapped.window.clone();
                        state.niri.layout.toggle_maximized(&window);
                        state.niri.queue_redraw_all();
                    }
                }
                if let WEnum::Unknown(_) = gesture {
                    resource.post_error(
                        gtk_surface1::Error::InvalidGesture,
                        "unknown titlebar gesture",
                    );
                }
            }
            // The compositor has no global menu, compositor window menu, or
            // legacy timestamp activation policy. Modal is an advisory hint;
            // xdg parent placement and existing input policy continue to apply.
            gtk_surface1::Request::SetDbusProperties { .. }
            | gtk_surface1::Request::SetModal
            | gtk_surface1::Request::UnsetModal
            | gtk_surface1::Request::Present { .. } => {}
        }
    }
    fn destroyed(
        &self,
        _state: &mut State,
        _client: smithay::reexports::wayland_server::backend::ClientId,
        resource: &GtkSurface1,
    ) {
        self.clear(resource);
    }
}
impl SurfaceData {
    fn clear(&self, resource: &GtkSurface1) {
        with_states(&self.surface, |states| {
            if let Some(data) = states.data_map.get::<SurfaceState>() {
                let mut annotation = data.0.lock().unwrap();
                annotation.remove(resource);
            }
        });
    }
}
fn valid_endpoint(bus_name: String, object_path: String) -> Option<AccessibilityEndpoint> {
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
    (valid_name && valid_path).then_some(AccessibilityEndpoint {
        bus_name,
        object_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gtk_one_never_receives_directional_states_added_in_two() {
        assert_eq!(tiled_states(true, 1), 1u32.to_ne_bytes());
        let states: Vec<_> = tiled_states(true, 2)
            .chunks_exact(4)
            .map(|word| u32::from_ne_bytes(word.try_into().unwrap()))
            .collect();
        assert_eq!(states, [1, 2, 3, 4, 5]);
        assert!(tiled_states(false, 7).is_empty());
    }
    #[test]
    fn accepts_only_unique_bus_names_and_valid_bounded_paths() {
        assert!(valid_endpoint(":1.32".into(), "/org/a11y/window_1".into()).is_some());
        for name in ["org.example.App", ":1", ":.1", ":1..3", ":1/2.3"] {
            assert!(valid_endpoint(name.into(), "/window".into()).is_none());
        }
        for path in ["", "window", "/window/", "/window//child", "/bad-path"] {
            assert!(valid_endpoint(":1.2".into(), path.into()).is_none());
        }
    }
    #[test]
    fn every_replacement_and_retirement_changes_generation() {
        let mut annotation = Annotation::default();
        annotation.attach(1u64);
        annotation.announce(&1, ":1.2".into(), "/window".into());
        let first = annotation.generation;
        annotation.announce(&1, ":1.2".into(), "/window".into());
        assert_ne!(annotation.generation, first);
        let second = annotation.generation;
        annotation.retire();
        assert_ne!(annotation.generation, second);
        assert!(annotation.read().unwrap().1.is_none());
    }
    #[test]
    fn invalid_announcement_retires_the_old_root_and_refuses_application_fallback() {
        let mut annotation = Annotation::default();
        annotation.attach(1u64);
        annotation.announce(&1, ":1.2".into(), "/window".into());
        let generation = annotation.generation;
        annotation.announce(&1, "org.example.App".into(), "/window".into());
        assert_ne!(annotation.generation, generation);
        assert_eq!(
            annotation.read().unwrap_err(),
            "native surface announced an invalid accessibility endpoint"
        );
        annotation.retire();
        assert!(annotation.read().unwrap().1.is_none());
    }
    #[test]
    fn duplicate_wrappers_refuse_binding_and_release_removes_only_its_owner() {
        let mut annotation = Annotation::default();
        annotation.attach(1u64);
        annotation.announce(&1, ":1.2".into(), "/first".into());
        annotation.attach(2);
        annotation.announce(&2, ":1.2".into(), "/second".into());
        assert_eq!(annotation.wrappers.len(), 2);
        assert_eq!(
            annotation.read().unwrap_err(),
            "multiple GTK wrappers make this surface's accessibility binding ambiguous"
        );
        let before = annotation.generation;
        annotation.remove(&1);
        assert_ne!(annotation.generation, before);
        assert_eq!(
            annotation.read().unwrap().1,
            Some(AccessibilityEndpoint {
                bus_name: ":1.2".into(),
                object_path: "/second".into()
            })
        );
        let remaining_generation = annotation.generation;
        annotation.remove(&1);
        annotation.announce(&1, ":1.2".into(), "/stale".into());
        assert_eq!(annotation.generation, remaining_generation);
        assert_eq!(annotation.read().unwrap().1.unwrap().object_path, "/second");
    }
    #[test]
    fn unmap_clears_every_wrapper_before_one_is_destroyed() {
        let mut annotation = Annotation::default();
        annotation.attach(1u64);
        annotation.announce(&1, ":1.2".into(), "/first".into());
        annotation.attach(2);
        annotation.announce(&2, ":1.2".into(), "/second".into());
        let before = annotation.generation;
        annotation.retire();
        assert_ne!(annotation.generation, before);
        annotation.remove(&1);
        assert!(annotation.read().unwrap().1.is_none());
        annotation.remove(&2);
        assert!(annotation.read().unwrap().1.is_none());
    }
}
