use super::*;
use niri_ipc::InputCoordination;
use smithay::reexports::wayland_server::{
    protocol::{wl_keyboard, wl_pointer},
    Resource,
};
use smithay::wayland::compositor::CompositorHandler;

impl State {
    pub fn input_coordination_snapshot(&self) -> InputCoordination {
        let mut human_owned_windows = Vec::new();
        self.niri.layout.with_windows(|window, _, _, _| {
            if self.human_owns_surface(window.toplevel().wl_surface()) {
                human_owned_windows.push(window.id().get());
            }
        });
        human_owned_windows.sort_unstable();
        InputCoordination {
            revision: self.niri.input_coordination.revision,
            human_owned_windows,
        }
    }

    pub(super) fn agent_input_failure(&self, reason: String) -> Result<Response, String> {
        if let Some(reason) = reason.strip_prefix("paused: ") {
            Ok(Response::AgentInputPaused {
                reason: reason.into(),
                coordination: self.input_coordination_snapshot(),
            })
        } else {
            Err(reason)
        }
    }

    pub(super) fn human_owns_surface(&self, surface: &WlSurface) -> bool {
        surface
            .client()
            .is_some_and(|client| self.niri.input_coordination.human_owns(&client.id()))
    }

    pub(super) fn require_agent_client(
        &mut self,
        agent: &AgentSeat,
        surface: &WlSurface,
    ) -> Result<(), String> {
        let root = self.niri.find_root_shell_surface(surface);
        if self
            .niri
            .layout
            .find_window_and_output(&root)
            .is_some_and(|(window, _)| self.external_target_unresolved(window.id().get()) || self.semantic_target_unresolved(window.id().get()))
        {
            return Err("paused: target has an unresolved external effect".into());
        }
        let connection = surface.client().ok_or("target client disconnected")?;
        let client = connection.id();
        if self.niri.input_coordination.human_owns(&client) {
            return Err("paused: human owns this application".into());
        }
        if agent
            .seat
            .get_pointer()
            .unwrap()
            .client_pointers(&connection)
            .next()
            .is_some()
            && agent
                .seat
                .get_keyboard()
                .unwrap()
                .client_keyboards(&connection)
                .next()
                .is_some()
        {
            return Ok(());
        }
        let same_client = |surface: &WlSurface| surface.client().is_some_and(|c| c.id() == client);
        let mut idle = Vec::new();
        for (&peer, other) in &self.niri.agent_seats {
            if other.target.as_ref().is_some_and(same_client)
                || other.contents.as_ref().is_some_and(|(s, _)| same_client(s))
            {
                if !other.keys.is_empty() || !other.buttons.is_empty() {
                    return Err("paused: another agent holds input in this application".into());
                }
                if peer != agent.peer {
                    idle.push(peer);
                }
            }
        }
        for peer in idle {
            if let Some(mut other) = self.niri.agent_seats.remove(&peer) {
                other.release_shared(self);
                self.niri.agent_seats.insert(peer, other);
            }
        }
        Ok(())
    }

    pub(crate) fn human_pointer_activity(&mut self, surface: Option<WlSurface>) {
        let keys_held = !self
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .pressed_keys()
            .is_empty();
        let client = surface.and_then(|s| s.client()).map(|c| c.id());
        self.semantic_input_activity();
        self.browser_human_activity(client.as_ref());
        if self
            .niri
            .input_coordination
            .pointer_activity(client, keys_held)
        {
            self.publish_input_coordination();
        }
        self.suspend_conflicting_agents();
    }

    pub(crate) fn human_pointer_focus_changed(&mut self, surface: Option<WlSurface>) {
        let client = surface.and_then(|s| s.client()).map(|c| c.id());
        self.semantic_input_activity();
        self.browser_human_activity(client.as_ref());
        if self.niri.input_coordination.pointer_focus_changed(client) {
            self.publish_input_coordination();
        }
        self.suspend_conflicting_agents();
    }

    pub(crate) fn yield_human_input(&mut self) -> Result<(), String> {
        if !self
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .pressed_keys()
            .is_empty()
            || self.niri.seat.get_pointer().unwrap().is_grabbed()
            || !self.niri.suppressed_buttons.is_empty()
        {
            return Err("cannot yield while human input is held".into());
        }
        if self.niri.input_coordination.yield_human() {
            self.publish_input_coordination();
        }
        Ok(())
    }

    pub(crate) fn human_keyboard_activity(&mut self) {
        let client = self
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .current_focus()
            .and_then(|s| s.client())
            .map(|c| c.id());
        self.semantic_input_activity();
        self.browser_human_activity(client.as_ref());
        if self.niri.input_coordination.keyboard_activity(client) {
            self.publish_input_coordination();
        }
        self.suspend_conflicting_agents();
    }

    pub(crate) fn publish_input_coordination(&self) {
        if let Some(server) = &self.niri.ipc_server {
            server.send_input_coordination(self.input_coordination_snapshot());
        }
    }

    pub(super) fn suspend_conflicting_agents(&mut self) {
        let peers: Vec<_> = self
            .niri
            .agent_seats
            .iter()
            .filter_map(|(&peer, agent)| {
                (agent
                    .target
                    .as_ref()
                    .is_some_and(|s| self.human_owns_surface(s))
                    || agent
                        .contents
                        .as_ref()
                        .is_some_and(|(s, _)| self.human_owns_surface(s)))
                .then_some(peer)
            })
            .collect();
        for peer in peers {
            let _ = self.with_agent(peer, |state, agent| {
                let visible = agent.visible;
                let contents = agent.contents.clone();
                let arrival = agent.arrival.take();
                agent.reset(state);
                agent.visible = visible;
                agent.contents = contents;
                if let Some(arrival) = arrival {
                    let _ =
                        arrival.reply.try_send(state.agent_input_failure(
                            "paused: human took over this application".into(),
                        ));
                }
                state.niri.queue_redraw_all();
            });
        }
    }
}

impl AgentSeat {
    pub(super) fn shared_pointer_motion(
        &mut self,
        state: &mut State,
        location: Point<f64, Logical>,
    ) -> Result<bool, String> {
        let focus = if self.buttons.is_empty() {
            self.contents.clone()
        } else {
            self.shared_pointer
                .clone()
                .or_else(|| self.contents.clone())
        };
        let Some((surface, origin)) = focus else {
            self.release_shared_pointer(state);
            return Ok(false);
        };
        let client = surface.client().ok_or("target client disconnected")?;
        if self
            .seat
            .get_pointer()
            .unwrap()
            .client_pointers(&client)
            .next()
            .is_some()
        {
            self.release_shared_pointer(state);
            return Ok(false);
        }
        let resources: Vec<_> = state
            .niri
            .seat
            .get_pointer()
            .unwrap()
            .client_pointers(&client)
            .collect();
        if resources.is_empty() {
            return Err("target client has not bound a usable pointer".into());
        }
        let changed = self
            .shared_pointer
            .as_ref()
            .is_none_or(|(old, _)| old != &surface);
        if changed {
            self.release_shared_pointer(state);
        }
        let scale = state.client_compositor_state(&client).client_scale();
        let relative = location - origin;
        let serial = SERIAL_COUNTER.next_serial();
        for pointer in resources {
            if changed {
                pointer.enter(
                    serial.into(),
                    &surface,
                    relative.x * scale,
                    relative.y * scale,
                );
            }
            pointer.motion(
                InputTime::now().millis(),
                relative.x * scale,
                relative.y * scale,
            );
            if pointer.version() >= 5 {
                pointer.frame();
            }
        }
        self.shared_pointer = Some((surface, origin));
        Ok(true)
    }

    pub(super) fn shared_button(
        &self,
        state: &State,
        button: u32,
        pressed: bool,
    ) -> Result<bool, String> {
        let Some((surface, _)) = &self.shared_pointer else {
            return Ok(false);
        };
        let client = surface.client().ok_or("target client disconnected")?;
        let serial = SERIAL_COUNTER.next_serial();
        for pointer in state
            .niri
            .seat
            .get_pointer()
            .unwrap()
            .client_pointers(&client)
        {
            pointer.button(
                serial.into(),
                InputTime::now().millis(),
                button,
                if pressed {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                },
            );
            if pointer.version() >= 5 {
                pointer.frame();
            }
        }
        Ok(true)
    }

    pub(super) fn shared_axis(
        &self,
        state: &State,
        horizontal: f64,
        vertical: f64,
    ) -> Result<bool, String> {
        let Some((surface, _)) = &self.shared_pointer else {
            return Ok(false);
        };
        let client = surface.client().ok_or("target client disconnected")?;
        let scale = state.client_compositor_state(&client).client_scale();
        for pointer in state
            .niri
            .seat
            .get_pointer()
            .unwrap()
            .client_pointers(&client)
        {
            if pointer.version() >= 5 {
                pointer.axis_source(wl_pointer::AxisSource::Continuous);
            }
            if horizontal != 0. {
                if pointer.version() >= 9 {
                    pointer.axis_relative_direction(
                        wl_pointer::Axis::HorizontalScroll,
                        wl_pointer::AxisRelativeDirection::Identical,
                    );
                }
                pointer.axis(
                    InputTime::now().millis(),
                    wl_pointer::Axis::HorizontalScroll,
                    horizontal * scale,
                );
            }
            if vertical != 0. {
                if pointer.version() >= 9 {
                    pointer.axis_relative_direction(
                        wl_pointer::Axis::VerticalScroll,
                        wl_pointer::AxisRelativeDirection::Identical,
                    );
                }
                pointer.axis(
                    InputTime::now().millis(),
                    wl_pointer::Axis::VerticalScroll,
                    vertical * scale,
                );
            }
            if pointer.version() >= 5 {
                pointer.frame();
            }
        }
        Ok(true)
    }

    pub(super) fn shared_keyboard_focus(&mut self, state: &mut State) -> Result<bool, String> {
        let surface = self
            .target
            .clone()
            .ok_or("focus a mapped window before typing")?;
        let client = surface.client().ok_or("target client disconnected")?;
        if self
            .seat
            .get_keyboard()
            .unwrap()
            .client_keyboards(&client)
            .next()
            .is_some()
        {
            self.release_shared_keyboard(state);
            return Ok(false);
        }
        let resources: Vec<_> = state
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .client_keyboards(&client)
            .collect();
        if resources.is_empty() {
            return Err("target client has not bound a usable keyboard".into());
        }
        if self.shared_keyboard.as_ref() != Some(&surface) {
            self.release_shared_keyboard(state);
            let serial = SERIAL_COUNTER.next_serial();
            for keyboard in resources {
                keyboard.enter(serial.into(), &surface, Vec::new());
            }
            self.shared_keyboard = Some(surface);
            self.shared_modifiers(state);
        }
        Ok(true)
    }

    pub(super) fn shared_modifiers(&self, state: &State) {
        let Some(client) = self.shared_keyboard.as_ref().and_then(|s| s.client()) else {
            return;
        };
        let mods = self
            .seat
            .get_keyboard()
            .unwrap()
            .modifier_state()
            .serialized;
        let serial = SERIAL_COUNTER.next_serial();
        for keyboard in state
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .client_keyboards(&client)
        {
            keyboard.modifiers(
                serial.into(),
                mods.depressed,
                mods.latched,
                mods.locked,
                mods.layout_effective,
            );
        }
    }

    pub(super) fn shared_key(
        &self,
        state: &State,
        evdev: u32,
        pressed: bool,
    ) -> Result<(), String> {
        let client = self
            .shared_keyboard
            .as_ref()
            .and_then(|s| s.client())
            .ok_or("target client disconnected")?;
        self.shared_modifiers(state);
        let serial = SERIAL_COUNTER.next_serial();
        for keyboard in state
            .niri
            .seat
            .get_keyboard()
            .unwrap()
            .client_keyboards(&client)
        {
            keyboard.key(
                serial.into(),
                InputTime::now().millis(),
                evdev,
                if pressed {
                    wl_keyboard::KeyState::Pressed
                } else {
                    wl_keyboard::KeyState::Released
                },
            );
        }
        Ok(())
    }

    pub(super) fn release_shared_pointer(&mut self, state: &State) {
        let Some((surface, _)) = self.shared_pointer.take() else {
            return;
        };
        let Some(client) = surface.client() else {
            return;
        };
        let human = state.niri.seat.get_pointer().unwrap();
        let serial = SERIAL_COUNTER.next_serial();
        let location = human.current_location();
        let under = state.niri.contents_under(location);
        let restore = under
            .surface
            .filter(|(s, _)| s.client().is_some_and(|c| c.id() == client.id()));
        let scale = state.client_compositor_state(&client).client_scale();
        for pointer in human.client_pointers(&client) {
            for &button in &self.buttons {
                pointer.button(
                    serial.into(),
                    InputTime::now().millis(),
                    button,
                    wl_pointer::ButtonState::Released,
                );
            }
            if !self.buttons.is_empty() && pointer.version() >= 5 {
                pointer.frame();
            }
            pointer.leave(serial.into(), &surface);
            if let Some((surface, origin)) = &restore {
                let relative = location - *origin;
                pointer.enter(
                    serial.into(),
                    surface,
                    relative.x * scale,
                    relative.y * scale,
                );
            }
            if pointer.version() >= 5 {
                pointer.frame();
            }
        }
    }

    pub(super) fn release_shared_keyboard(&mut self, state: &State) {
        let Some(surface) = self.shared_keyboard.take() else {
            return;
        };
        let Some(client) = surface.client() else {
            return;
        };
        let human = state.niri.seat.get_keyboard().unwrap();
        let restore = human
            .current_focus()
            .filter(|s| s.client().is_some_and(|c| c.id() == client.id()));
        let keys = human
            .pressed_keys()
            .into_iter()
            .filter(|key| !state.niri.suppressed_keys.contains(key))
            .flat_map(|key| (u32::from(key) - 8).to_ne_bytes())
            .collect::<Vec<_>>();
        let mods = human.modifier_state().serialized;
        let serial = SERIAL_COUNTER.next_serial();
        for keyboard in human.client_keyboards(&client) {
            for &evdev in &self.keys {
                keyboard.key(
                    serial.into(),
                    InputTime::now().millis(),
                    evdev,
                    wl_keyboard::KeyState::Released,
                );
            }
            keyboard.leave(serial.into(), &surface);
            if let Some(surface) = &restore {
                keyboard.enter(serial.into(), surface, keys.clone());
            }
            keyboard.modifiers(
                serial.into(),
                mods.depressed,
                mods.latched,
                mods.locked,
                mods.layout_effective,
            );
        }
    }

    pub(super) fn release_shared(&mut self, state: &State) {
        self.release_shared_pointer(state);
        self.release_shared_keyboard(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture::Fixture;

    #[test]
    fn takeover_cancels_arrival_and_keeps_cursor_visible() {
        let mut f = Fixture::with_agent_input();
        f.add_output(1, (1920, 1080));
        let client = f.add_client();
        let window = f.client(client).create_window();
        let surface = window.surface.clone();
        window.commit();
        f.double_roundtrip(client);
        let window = f.client(client).window(&surface);
        window.attach_new_buffer();
        window.ack_last_and_commit();
        f.double_roundtrip(client);
        let window = f.client(client).window(&surface);
        window.set_size(1000, 1000);
        window.commit();
        f.double_roundtrip(client);
        let target = f.niri().layout.focus().unwrap().id().get();
        let output = f.niri_output(1).name();
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Create {
                    name: "moving".into(),
                },
            )
            .unwrap();
        f.double_roundtrip(client);
        f.niri_state()
            .apply_agent_input(1, AgentInput::Focus { window_id: target })
            .unwrap();
        f.niri().clock.set_complete_instantly(true);
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Move {
                    output: output.clone(),
                    x: 500.,
                    y: 500.,
                },
            )
            .unwrap();
        f.niri().clock.set_complete_instantly(false);
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Move {
                    output,
                    x: 600.,
                    y: 600.,
                },
            )
            .unwrap();
        let (tx, rx) = async_channel::bounded(1);
        f.niri_state().request_agent_input(
            1,
            AgentInput::Coordinated {
                revision: 0,
                command: Box::new(AgentInput::Wait),
            },
            tx,
        );
        assert!(rx.try_recv().is_err());
        f.niri_state()
            .process_input_event(smithay::backend::input::InputEvent::<
                crate::tests::test_input_backend::TestInputBackend,
            >::PointerMotionAbsolute {
                event: crate::tests::test_input_backend::TestPointerMotionAbsoluteEvent {
                    x: 500.,
                    y: 500.,
                },
            });
        assert!(matches!(
            rx.try_recv().unwrap().unwrap(),
            Response::AgentInputPaused { .. }
        ));
        let agent = &f.niri().agent_seats[&1];
        assert!(agent.visible);
        assert!(agent.motion.is_none());
        assert!(agent.arrival.is_none());
        assert!(agent.keys.is_empty() && agent.buttons.is_empty());
    }
}
