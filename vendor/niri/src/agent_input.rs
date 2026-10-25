//! IPC-controlled logical seats. Each controller owns its seat state; dispatch selects that
//! state synchronously so Smithay can resolve handles without retaining retired seats forever.
//! DOJO_AGENT_INPUT=1 disables virtual keyboards and input methods;
//! agent input has no client cursors, popup grabs, interactive move/resize, DnD, or X11 support.

use std::collections::HashSet;
use std::io::Cursor;
use std::rc::Rc;
use std::time::Duration;

use anyhow::Context as _;
use calloop::timer::{TimeoutAction, Timer};
use niri_ipc::{AgentInput, AgentInputState, Response};
use smithay::backend::allocator::Fourcc;
use smithay::backend::input::{Axis, AxisSource, ButtonState, InputTime, KeyState, Keycode};
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::input::keyboard::FilterResult;
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::input::{Seat, SeatState};
use smithay::output::Output;
use smithay::reexports::wayland_server::backend::GlobalId;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Scale, Transform, SERIAL_COUNTER};

mod coordination;
pub(crate) use coordination::Coordination;
mod motion;
mod shared;
use motion::Motion;

use crate::niri::{LockState, Niri, OutputRenderElements, State};
use crate::render_helpers::renderer::NiriRenderer;
use crate::utils::get_monotonic_time;

pub struct AgentSeat {
    peer: u64,
    owner: Option<Rc<peer::KernelPeer>>,
    pub seat: Seat<State>,
    state: SeatState<State>,
    target: Option<WlSurface>,
    contents: Option<(WlSurface, Point<f64, Logical>)>,
    keys: HashSet<u32>,
    buttons: HashSet<u32>,
    visible: bool,
    cursor: MemoryRenderBuffer,
    motion: Option<Motion>,
    captured_geometry: Option<niri_ipc::CapturedInputGeometry>,
    arrival: Option<Arrival>,
    shared_pointer: Option<(WlSurface, Point<f64, Logical>)>,
    shared_keyboard: Option<WlSurface>,
}

pub(crate) mod peer;

impl State {
    pub(crate) fn bind_agent_owner(&mut self, id: u64, owner: Option<Rc<peer::KernelPeer>>) {
        if let Some(seat) = self.niri.agent_seats.get_mut(&id) {
            if seat.owner.is_none() {
                seat.owner = owner;
            }
        }
    }
    pub(crate) fn agent_peer_owned_by(&self, id: u64, pid: u32, start: u64) -> bool {
        self.niri
            .agent_seats
            .get(&id)
            .and_then(|seat| seat.owner.as_ref())
            .is_some_and(|owner| owner.matches_process(pid, start))
    }
}

struct Arrival {
    command: AgentInput,
    reply: async_channel::Sender<niri_ipc::Reply>,
}

pub fn enabled() -> bool {
    std::env::var_os("DOJO_AGENT_INPUT").is_some_and(|value| value == "1")
}

fn validate(command: &AgentInput) -> Result<(), String> {
    match command {
        AgentInput::Captured { geometry, command } => {
            if geometry.rect.2 <= 0.
                || geometry.rect.3 <= 0.
                || ![
                    geometry.rect.0,
                    geometry.rect.1,
                    geometry.rect.2,
                    geometry.rect.3,
                    geometry.output_geometry.scale,
                ]
                .iter()
                .all(|v| v.is_finite())
            {
                return Err("invalid captured geometry".into());
            }
            if matches!(
                **command,
                AgentInput::Create { .. }
                    | AgentInput::Destroy
                    | AgentInput::Coordinated { .. }
                    | AgentInput::Captured { .. }
            ) {
                return Err("captured envelope requires one input command".into());
            }
            if let AgentInput::Focus { window_id } = **command {
                if window_id != geometry.window_id {
                    return Err("captured focus target mismatch".into());
                }
            }
            if let AgentInput::Targeted { window_id, .. } = **command {
                if window_id != geometry.window_id {
                    return Err("captured target mismatch".into());
                }
            }
            return validate(command);
        }
        AgentInput::Coordinated { command, .. } => return validate(command),
        AgentInput::Targeted { command, .. } => {
            if !matches!(
                **command,
                AgentInput::Button { .. }
                    | AgentInput::Axis { .. }
                    | AgentInput::Key { .. }
                    | AgentInput::TextKey { .. }
                    | AgentInput::Move { .. }
            ) {
                return Err(
                    "Targeted accepts only Button, Axis, Key, TextKey, or Move commands".into(),
                );
            }
            return validate(command);
        }
        AgentInput::Create { name } => {
            if name.is_empty()
                || name.len() > 32
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
            {
                return Err(
                    "agent name must contain 1–32 ASCII letters, digits, '-' or '_'".into(),
                );
            }
        }
        AgentInput::Move { x, y, .. } | AgentInput::Jump { x, y, .. } if !x.is_finite() || !y.is_finite() => {
            return Err("pointer coordinates must be finite".into());
        }
        AgentInput::Button { button, .. } if !(0x110..=0x117).contains(button) => {
            return Err("button must be a Linux mouse button code between 272 and 279".into());
        }
        AgentInput::Key { evdev, .. } | AgentInput::TextKey { evdev, .. }
            if !(1..=255).contains(evdev) =>
        {
            return Err("evdev keyboard code must be between 1 and 255".into());
        }
        AgentInput::Axis {
            horizontal,
            vertical,
        } if !horizontal.is_finite()
            || !vertical.is_finite()
            || horizontal.abs() > 10000.
            || vertical.abs() > 10000. =>
        {
            return Err("scroll amounts must be finite and at most 10000 pixels".into());
        }
        _ => {}
    }
    Ok(())
}

fn is_release(command: &AgentInput) -> bool {
    match command {
        AgentInput::Targeted { command, .. } => is_release(command),
        AgentInput::Key {
            state: AgentInputState::Released,
            ..
        }
        | AgentInput::TextKey {
            state: AgentInputState::Released,
            ..
        }
        | AgentInput::Button {
            state: AgentInputState::Released,
            ..
        } => true,
        _ => false,
    }
}

fn is_move(command: &AgentInput) -> bool {
    match command {
        AgentInput::Move { .. } | AgentInput::Jump { .. } => true,
        AgentInput::Captured { command, .. }
        | AgentInput::Coordinated { command, .. }
        | AgentInput::Targeted { command, .. } => is_move(command),
        _ => false,
    }
}

// Keep all wrappers intact until arrival, then check ownership and target at dispatch.
fn deferred(command: &AgentInput) -> bool {
    match command {
        AgentInput::Captured { command, .. }
        | AgentInput::Coordinated { command, .. }
        | AgentInput::Targeted { command, .. } => deferred(command),
        AgentInput::Wait | AgentInput::Button { .. } | AgentInput::Axis { .. } => true,
        _ => false,
    }
}

impl State {
    pub(crate) fn recolor_agent_cursors(&mut self, color: niri_config::Color) {
        for agent in self.niri.agent_seats.values_mut() {
            match font_pointer_buffer(color) {
                Ok(cursor) => agent.cursor = cursor,
                Err(error) => warn!("cannot recolor agent cursor: {error:#}"),
            }
        }
        self.niri.queue_redraw_all();
    }

    pub fn apply_agent_input(
        &mut self,
        peer: u64,
        command: AgentInput,
    ) -> Result<Response, String> {
        if let AgentInput::Coordinated { revision, command } = command {
            if revision != self.niri.input_coordination.revision {
                return self.agent_input_failure("paused: input ownership changed".into());
            }
            return self.apply_agent_input(peer, *command);
        }
        self.check_agent_input(&command)?;
        self.refresh_agents();
        if let AgentInput::Create { name } = command {
            if self.niri.agent_seats.contains_key(&peer) {
                return Err("this connection already owns an agent seat".into());
            }
            if self.niri.agent_seats.len() >= 16 {
                return Err("too many live agent controllers".into());
            }
            let cursor = font_pointer_buffer(self.niri.config.borrow().cursor.agent_color)
                .map_err(|error| format!("cannot load agent pointer icon: {error:#}"))?;
            let seat_name = format!("dojo-agent-{peer}-{name}");
            let mut state = SeatState::new();
            let mut seat = state.new_wl_seat(&self.niri.display_handle, seat_name.clone());
            let config = self.niri.config.borrow();
            if let Err(error) = seat.add_keyboard(
                config.input.keyboard.xkb.to_xkb_config(),
                config.input.keyboard.repeat_delay.into(),
                config.input.keyboard.repeat_rate.into(),
            ) {
                drop(config);
                if let Some(global) = seat.global() {
                    self.retire_agent_global(global);
                }
                return Err(format!("cannot create agent keyboard: {error}"));
            }
            seat.add_pointer();
            self.niri.agent_seats.insert(
                peer,
                AgentSeat {
                    peer,
                    owner: None,
                    seat,
                    state,
                    target: None,
                    contents: None,
                    keys: HashSet::new(),
                    buttons: HashSet::new(),
                    visible: false,
                    cursor,
                    motion: None,
                    captured_geometry: None,
                    arrival: None,
                    shared_pointer: None,
                    shared_keyboard: None,
                },
            );
            return Ok(Response::AgentInputCreated {
                seat_name,
                peer_id: peer,
            });
        }
        if matches!(command, AgentInput::Destroy) {
            self.destroy_agent(peer);
            return Ok(Response::Handled);
        }
        let moving = is_move(&command);
        let result = self.with_agent(peer, |this, agent| this.dispatch_agent(agent, command))?;
        if let Err(reason) = result {
            return self.agent_input_failure(reason);
        }
        if moving && self.niri.agent_seats[&peer].motion.is_some() {
            Ok(Response::AgentInputMoving)
        } else {
            Ok(Response::Handled)
        }
    }

    fn check_agent_input(&self, command: &AgentInput) -> Result<(), String> {
        validate(command)?;
        if !self.niri.agent_input_enabled {
            return Err("agent input requires DOJO_AGENT_INPUT=1 on the compositor".into());
        }
        if !matches!(command, AgentInput::Reset | AgentInput::Destroy)
            && !matches!(self.niri.lock_state, LockState::Unlocked)
        {
            return Err("agent input is suspended while the session is locking or locked".into());
        }
        Ok(())
    }

    pub fn request_agent_input(
        &mut self,
        peer: u64,
        command: AgentInput,
        reply: async_channel::Sender<niri_ipc::Reply>,
    ) {
        if deferred(&command) {
            if let Err(error) = self.check_agent_input(&command) {
                let _ = reply.try_send(Err(error));
                return;
            }
            self.refresh_agents();
            if let Some(agent) = self.niri.agent_seats.get_mut(&peer) {
                if let Some(motion) = &agent.motion {
                    let remaining = motion.remaining(get_monotonic_time());
                    agent.arrival = Some(Arrival { command, reply });
                    // Frame callbacks drive movement; this one-shot also completes waits when
                    // an output stops presenting frames.
                    if let Err(error) = self.niri.event_loop.insert_source(
                        Timer::from_duration(remaining),
                        move |_, _, state| {
                            state.refresh_agents();
                            TimeoutAction::Drop
                        },
                    ) {
                        let agent = self.niri.agent_seats.get_mut(&peer).unwrap();
                        let arrival = agent.arrival.take().unwrap();
                        let _ = arrival
                            .reply
                            .try_send(Err(format!("cannot schedule pointer arrival: {error}")));
                    }
                    return;
                }
            }
        }
        let result = self.apply_agent_input(peer, command);
        let _ = reply.try_send(result);
    }

    pub(crate) fn with_agent<T>(
        &mut self,
        peer: u64,
        apply: impl FnOnce(&mut Self, &mut AgentSeat) -> T,
    ) -> Result<T, String> {
        let mut agent = self
            .niri
            .agent_seats
            .remove(&peer)
            .ok_or_else(|| "create an agent seat on this connection first".to_string())?;
        self.niri.agent_dispatch_state = Some(std::mem::take(&mut agent.state));
        let result = apply(self, &mut agent);
        agent.state = self.niri.agent_dispatch_state.take().unwrap();
        self.niri.agent_seats.insert(peer, agent);
        Ok(result)
    }

    pub(crate) fn require_capture_geometry(
        &self,
        geometry: &niri_ipc::CapturedInputGeometry,
    ) -> Result<(), String> {
        if geometry.coordination_revision != self.niri.input_coordination.revision {
            return Err("paused: input ownership changed".into());
        }
        if geometry.compositor_pid != std::process::id() {
            return Err("capture compositor generation changed".into());
        }
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", std::process::id()))
            .map_err(|e| e.to_string())?;
        let start = stat
            .rsplit_once(')')
            .and_then(|(_, v)| v.split_whitespace().nth(19))
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or("compositor generation unavailable")?;
        if start != geometry.compositor_start_time {
            return Err("capture compositor generation changed".into());
        }
        let output = self
            .niri
            .global_space
            .outputs()
            .find(|o| o.name() == geometry.output)
            .ok_or("capture output disappeared")?;
        if crate::utils::logical_output(output) != geometry.output_geometry {
            return Err("capture output geometry changed".into());
        }
        let placements = self
            .niri
            .layout
            .capture_placements(output)
            .map_err(str::to_owned)?;
        let (window, rect, _) = placements
            .into_iter()
            .find(|(window, _, _)| window.id().get() == geometry.window_id)
            .ok_or("capture window disappeared")?;
        let actual_pid = window.credentials().and_then(|c| u32::try_from(c.pid).ok());
        let actual_rect = (rect.loc.x, rect.loc.y, rect.size.w, rect.size.h);
        if let Some(reason) =
            capture_window_mismatch(geometry.pid, actual_pid, geometry.rect, actual_rect)
        {
            return Err(reason);
        }
        Ok(())
    }

    /// Resolve compositor-owned target authority immediately before a bounded external send.
    pub(crate) fn resolve_external_target(
        &mut self,
        peer: u64,
        revision: u64,
        window_id: u64,
    ) -> Result<(WlSurface, u32), String> {
        if revision != self.niri.input_coordination.revision {
            return Err("paused: input ownership changed".into());
        }
        if !self.niri.agent_input_enabled || !matches!(self.niri.lock_state, LockState::Unlocked) {
            return Err("external mutation requires an enabled unlocked agent seat".into());
        }
        let mut target = None;
        self.niri.layout.with_windows(|window, output, _, _| {
            if window.id().get() == window_id && output.is_some() {
                if let Some(credentials) = window.credentials() {
                    target = Some((window.toplevel().wl_surface().clone(), credentials.pid));
                }
            }
        });
        let (surface, pid) = target.ok_or("external target has no mapped live process identity")?;
        let pid = u32::try_from(pid).map_err(|_| "external target has invalid process identity")?;
        self.with_agent(peer, |this, agent| {
            this.require_agent_client(agent, &surface)
        })??;
        Ok((surface, pid))
    }

    fn dispatch_agent(&mut self, agent: &mut AgentSeat, command: AgentInput) -> Result<(), String> {
        self.semantic_input_activity();
        self.dispatch_agent_targeted(agent, command, None)
    }

    fn require_agent_window(
        &self,
        surface: Option<&WlSurface>,
        window_id: u64,
    ) -> Result<(), String> {
        let mapped = surface.and_then(|surface| {
            // This resolves subsurfaces and popup ancestry, never client identity.
            let root = self.niri.find_root_shell_surface(surface);
            self.niri
                .layout
                .find_window_and_output(&root)
                .filter(|(_, output)| output.is_some())
                .map(|(window, _)| window.id().get())
        });
        if mapped != Some(window_id) {
            return Err(format!(
                "agent target mismatch: expected mapped window {window_id}, found {mapped:?}"
            ));
        }
        Ok(())
    }

    fn require_agent_pointer_window(
        &mut self,
        agent: &mut AgentSeat,
        window_id: u64,
    ) -> Result<(), String> {
        let pointer = agent.seat.get_pointer().unwrap();
        let location = pointer.current_location();
        // Re-hit-test now, not at capture time or the previous animation frame.
        let under = self.niri.contents_under(location);
        self.require_agent_window(
            under.surface.as_ref().map(|(surface, _)| surface),
            window_id,
        )?;
        let surface = &under.surface.as_ref().unwrap().0;
        self.require_agent_client(agent, surface)?;
        // An implicit button grab can retain another window even when the hit test matches.
        if let Some((surface, _)) = &agent.shared_pointer {
            if !agent.buttons.is_empty() {
                self.require_agent_window(Some(surface), window_id)?;
            }
        }
        agent.move_to(self, location);
        self.require_agent_window(pointer.current_focus().as_ref(), window_id)
    }

    fn dispatch_agent_targeted(
        &mut self,
        agent: &mut AgentSeat,
        command: AgentInput,
        expected: Option<u64>,
    ) -> Result<(), String> {
        let pointer = agent.seat.get_pointer().unwrap();
        let keyboard = agent.seat.get_keyboard().unwrap();
        let instant = matches!(&command, AgentInput::Jump { .. });
        let redraw_all = !matches!(
            &command,
            AgentInput::Move { .. } | AgentInput::Jump { .. } | AgentInput::Wait
        );
        match command {
            AgentInput::Captured { geometry, command } => {
                if let Err(reason) = self.require_capture_geometry(&geometry) {
                    if is_release(&command) {
                        let _ =
                            self.dispatch_agent_targeted(agent, *command, Some(geometry.window_id));
                    }
                    return Err(reason);
                }
                agent.captured_geometry = Some(geometry.clone());
                return self.dispatch_agent_targeted(agent, *command, Some(geometry.window_id));
            }
            AgentInput::Coordinated { revision, command } => {
                if revision != self.niri.input_coordination.revision {
                    return Err("paused: input ownership changed".into());
                }
                return self.dispatch_agent_targeted(agent, *command, expected);
            }
            AgentInput::Targeted { window_id, command } => {
                return self.dispatch_agent_targeted(agent, *command, Some(window_id));
            }
            AgentInput::Focus { window_id } => {
                let mut target = None;
                self.niri.layout.with_windows(|mapped, output, _, _| {
                    if mapped.id().get() == window_id && output.is_some() {
                        target = Some(mapped.toplevel().wl_surface().clone());
                    }
                });
                let target = target.ok_or_else(|| "no mapped window with that ID".to_string())?;
                self.require_agent_client(agent, &target)?;
                agent.set_target(self, target);
                agent.shared_keyboard_focus(self)?;
            }
            AgentInput::Move { output, x, y } | AgentInput::Jump { output, x, y } => {
                let output = self
                    .niri
                    .global_space
                    .outputs()
                    .find(|candidate| candidate.name() == output)
                    .cloned()
                    .ok_or_else(|| "no connected output with that name".to_string())?;
                let geometry = self.niri.global_space.output_geometry(&output).unwrap();
                if x < 0. || y < 0. || x >= geometry.size.w as f64 || y >= geometry.size.h as f64 {
                    return Err("pointer coordinates must be inside the output".into());
                }
                let location = geometry.loc.to_f64() + Point::from((x, y));
                if let Some(window_id) = expected {
                    let under = self.niri.contents_under(location);
                    self.require_agent_window(
                        under.surface.as_ref().map(|(surface, _)| surface),
                        window_id,
                    )?;
                    let surface = &under.surface.as_ref().unwrap().0;
                    self.require_agent_client(agent, surface)?;
                    if !agent.buttons.is_empty() {
                        self.require_agent_pointer_window(agent, window_id)?;
                    }
                }
                let now = get_monotonic_time();
                let from = if agent.visible {
                    pointer.current_location()
                } else {
                    let human = self.niri.seat.get_pointer().unwrap().current_location();
                    if self.niri.output_under(human).is_some() {
                        human
                    } else {
                        location
                    }
                };
                let velocity = agent
                    .motion
                    .as_ref()
                    .map_or(Point::from((0., 0.)), |motion| motion.sample(now).1);
                agent.motion = if instant
                    || self.niri.clock.should_complete_instantly()
                    || (location.x - from.x).hypot(location.y - from.y) < 1.
                {
                    None
                } else {
                    Some(Motion::new(from, location, velocity, now))
                };
                agent.move_to(
                    self,
                    if agent.motion.is_some() {
                        from
                    } else {
                        location
                    },
                );
                agent.visible = true;
                if agent.motion.is_some() {
                    self.niri.queue_redraw(&output);
                }
            }
            AgentInput::Button { button, state } => {
                let pressed = state == AgentInputState::Pressed;
                if pressed {
                    if let Some(window_id) = expected {
                        self.require_agent_pointer_window(agent, window_id)?;
                    }
                    let surface = agent
                        .contents
                        .as_ref()
                        .map(|(surface, _)| surface)
                        .ok_or("move the agent pointer onto a client surface first")?;
                    self.require_agent_client(agent, surface)?;
                    agent.shared_pointer_motion(self, pointer.current_location())?;
                    if !agent.visible || pointer.current_focus().is_none() {
                        return Err("move the agent pointer onto a client surface first".into());
                    }
                    if !agent.buttons.insert(button) {
                        return Ok(());
                    }
                    if let Some(surface) = pointer.current_focus() {
                        let root = self.niri.find_root_shell_surface(&surface);
                        if self.niri.layout.find_window_and_output(&root).is_some() {
                            agent.set_target(self, root);
                        }
                    }
                } else if !agent.buttons.remove(&button) {
                    return Ok(());
                }
                if !agent.shared_button(self, button, pressed)? {
                    pointer.button(
                        self,
                        &ButtonEvent {
                            button,
                            state: if pressed {
                                ButtonState::Pressed
                            } else {
                                ButtonState::Released
                            },
                            serial: SERIAL_COUNTER.next_serial(),
                            time: InputTime::now(),
                        },
                    );
                    pointer.frame(self);
                }
            }
            AgentInput::TextKey { evdev, state } => {
                if state == AgentInputState::Pressed {
                    let config = self.niri.config.borrow();
                    let xkb = &config.input.keyboard.xkb;
                    let mods = keyboard.modifier_state().serialized;
                    if xkb.layout != "us"
                        || !xkb.variant.is_empty()
                        || xkb.file.is_some()
                        || xkb.options.as_ref().is_some_and(|v| !v.is_empty())
                        || !xkb.rules.is_empty()
                        || !xkb.model.is_empty()
                        || mods.locked != 0
                        || mods.latched != 0
                        || mods.layout_effective != 0
                        || agent.keys.iter().any(|code| *code != 42)
                    {
                        return Err("physical text requires plain US layout, no locked modifiers, and no other held keys".into());
                    }
                }
                return self.dispatch_agent_targeted(
                    agent,
                    AgentInput::Key { evdev, state },
                    expected,
                );
            }
            AgentInput::Key { evdev, state } => {
                if agent.target.as_ref().is_none_or(|surface| {
                    self.niri
                        .layout
                        .find_window_and_output(surface)
                        .is_none_or(|(_, output)| output.is_none())
                }) {
                    agent.reset(self);
                    return Err("focus a mapped window before typing".into());
                }
                let pressed = state == AgentInputState::Pressed;
                if pressed {
                    if let Some(window_id) = expected {
                        self.require_agent_window(agent.target.as_ref(), window_id)?;
                        self.require_agent_window(keyboard.current_focus().as_ref(), window_id)?;
                    }
                    self.require_agent_client(agent, agent.target.as_ref().unwrap())?;
                }
                let shared = agent.shared_keyboard_focus(self)?;
                if pressed {
                    if !agent.keys.insert(evdev) {
                        return Ok(());
                    }
                } else if !agent.keys.remove(&evdev) {
                    return Ok(());
                }
                keyboard.input::<(), _>(
                    self,
                    Keycode::from(evdev + 8),
                    if pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    SERIAL_COUNTER.next_serial(),
                    InputTime::now(),
                    |_, _, _| {
                        if shared {
                            FilterResult::Intercept(())
                        } else {
                            FilterResult::Forward
                        }
                    },
                );
                if shared {
                    agent.shared_key(self, evdev, pressed)?;
                }
            }
            AgentInput::Axis {
                horizontal,
                vertical,
            } => {
                if let Some(window_id) = expected {
                    self.require_agent_pointer_window(agent, window_id)?;
                }
                if !agent.visible || pointer.current_focus().is_none() {
                    return Err("move the agent pointer onto a client surface first".into());
                }
                let surface = agent
                    .contents
                    .as_ref()
                    .map(|(surface, _)| surface)
                    .ok_or("move the agent pointer onto a client surface first")?;
                self.require_agent_client(agent, surface)?;
                agent.shared_pointer_motion(self, pointer.current_location())?;
                if !agent.shared_axis(self, horizontal, vertical)? {
                    pointer.axis(
                        self,
                        AxisFrame::new(InputTime::now())
                            .source(AxisSource::Continuous)
                            .value(Axis::Horizontal, horizontal)
                            .value(Axis::Vertical, vertical),
                    );
                    pointer.frame(self);
                }
            }
            AgentInput::Wait => {}
            AgentInput::Reset => agent.reset(self),
            AgentInput::Create { .. } | AgentInput::Destroy => unreachable!(),
        }
        if redraw_all {
            self.niri.queue_redraw_all();
        }
        Ok(())
    }

    pub fn destroy_agent(&mut self, peer: u64) {
        let _ = self.with_agent(peer, |this, agent| {
            agent.reset(this);
            agent.seat.remove_keyboard();
            agent.seat.remove_pointer();
            if let Some(global) = agent.seat.global() {
                this.retire_agent_global(global);
            }
        });
        self.niri.agent_seats.remove(&peer);
    }

    fn retire_agent_global(&mut self, global: GlobalId) {
        self.niri
            .display_handle
            .disable_global::<State>(global.clone());
        let retired = global.clone();
        if self
            .niri
            .event_loop
            .insert_source(
                Timer::from_duration(Duration::from_secs(2)),
                move |_, _, state| {
                    state
                        .niri
                        .display_handle
                        .remove_global::<State>(retired.clone());
                    TimeoutAction::Drop
                },
            )
            .is_err()
        {
            self.niri.display_handle.remove_global::<State>(global);
        }
    }

    pub fn refresh_agents(&mut self) {
        let peers: Vec<_> = self.niri.agent_seats.keys().copied().collect();
        for peer in peers {
            let _ = self.with_agent(peer, |this, agent| {
                if !matches!(this.niri.lock_state, LockState::Unlocked)
                    || agent.target.as_ref().is_some_and(|target| {
                        this.niri
                            .layout
                            .find_window_and_output(target)
                            .is_none_or(|(_, output)| output.is_none())
                    })
                {
                    agent.reset(this);
                    return;
                }
                if agent
                    .captured_geometry
                    .as_ref()
                    .is_some_and(|geometry| this.require_capture_geometry(geometry).is_err())
                {
                    agent.reset(this);
                    return;
                }
                if agent.visible {
                    let pointer = agent.seat.get_pointer().unwrap();
                    let now = get_monotonic_time();
                    if let Some(motion) = &agent.motion {
                        if this.niri.output_under(motion.destination()).is_none() {
                            agent.reset(this);
                            return;
                        }
                        let location = motion.sample(now).0;
                        let done = motion.remaining(now).is_zero()
                            || this.niri.clock.should_complete_instantly();
                        let location = if done { motion.destination() } else { location };
                        if !agent.buttons.is_empty() {
                            let under = this.niri.contents_under(location);
                            let expected = agent.target.as_ref().and_then(|target| {
                                this.niri
                                    .layout
                                    .find_window_and_output(target)
                                    .map(|(window, _)| window.id().get())
                            });
                            if expected.is_none_or(|id| {
                                this.require_agent_window(
                                    under.surface.as_ref().map(|(surface, _)| surface),
                                    id,
                                )
                                .is_err()
                            }) {
                                agent.reset(this);
                                return;
                            }
                        }
                        agent.move_to(this, location);
                        if done {
                            if let Some((output, _)) = this.niri.output_under(location) {
                                let output = output.clone();
                                this.niri.queue_redraw(&output);
                            }
                            agent.motion = None;
                            if let Some(arrival) = agent.arrival.take() {
                                let result = this
                                    .dispatch_agent(agent, arrival.command)
                                    .map(|()| Response::Handled);
                                let result =
                                    result.or_else(|reason| this.agent_input_failure(reason));
                                let _ = arrival.reply.try_send(result);
                            }
                        }
                    }
                    let location = pointer.current_location();
                    if agent.motion.is_none() && this.niri.output_under(location).is_none() {
                        agent.reset(this);
                        return;
                    }
                    if pointer.current_focus().is_some_and(|surface| {
                        let root = this.niri.find_root_shell_surface(&surface);
                        this.niri.layout.find_window_and_output(&root).is_none()
                            && !this
                                .niri
                                .mapped_layer_surfaces
                                .keys()
                                .any(|layer| *layer.wl_surface() == root)
                    }) {
                        agent.reset(this);
                        return;
                    }
                    let under = this.niri.contents_under(location);
                    if agent.contents == under.surface {
                        return;
                    }
                    agent.move_to(this, location);
                }
            });
        }
    }

    pub fn reset_all_agents(&mut self) {
        let peers: Vec<_> = self.niri.agent_seats.keys().copied().collect();
        for peer in peers {
            let _ = self.with_agent(peer, |this, agent| agent.reset(this));
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        let peers: Vec<_> = self.niri.agent_seats.keys().copied().collect();
        let had_agents = !peers.is_empty();
        for peer in peers {
            self.destroy_agent(peer);
        }
        if had_agents {
            let _ = self.niri.display_handle.flush_clients();
        }
    }
}

impl AgentSeat {
    fn move_to(&mut self, state: &mut State, location: Point<f64, Logical>) {
        let under = state.niri.contents_under(location);
        let pointer = self.seat.get_pointer().unwrap();
        let previous = pointer.current_location();
        if !self.visible || previous != location {
            let old_output = self
                .visible
                .then(|| {
                    state
                        .niri
                        .output_under(previous)
                        .map(|(output, _)| output.clone())
                })
                .flatten();
            let new_output = state
                .niri
                .output_under(location)
                .map(|(output, _)| output.clone());
            for output in old_output.into_iter().chain(new_output) {
                state.niri.queue_redraw(&output);
            }
        }
        let allowed = under.surface.as_ref().is_some_and(|(surface, _)| {
            self.captured_geometry.as_ref().is_none_or(|geometry| {
                state
                    .require_agent_window(Some(surface), geometry.window_id)
                    .is_ok()
            }) && state.require_agent_client(self, surface).is_ok()
        });
        self.contents = if self.captured_geometry.is_some() && !allowed {
            None
        } else {
            under.surface.clone()
        };
        pointer.motion(
            state,
            if allowed { under.surface } else { None },
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time: InputTime::now(),
            },
        );
        pointer.frame(state);
        if allowed {
            let _ = self.shared_pointer_motion(state, location);
        } else {
            self.release_shared_pointer(state);
        }
    }

    fn set_target(&mut self, state: &mut State, target: WlSurface) {
        if self.target.as_ref() == Some(&target) {
            return;
        }
        self.release_keys(state);
        self.release_shared_keyboard(state);
        self.target = Some(target.clone());
        self.seat.get_keyboard().unwrap().set_focus(
            state,
            Some(target),
            SERIAL_COUNTER.next_serial(),
        );
    }

    fn release_keys(&mut self, state: &mut State) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let shared = self.shared_keyboard.is_some();
        for evdev in std::mem::take(&mut self.keys) {
            keyboard.input::<(), _>(
                state,
                Keycode::from(evdev + 8),
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_, _, _| {
                    if shared {
                        FilterResult::Intercept(())
                    } else {
                        FilterResult::Forward
                    }
                },
            );
            if shared {
                let _ = self.shared_key(state, evdev, false);
            }
        }
    }

    fn reset(&mut self, state: &mut State) {
        let shared_pointer = self.shared_pointer.is_some();
        self.release_keys(state);
        self.release_shared(state);
        self.motion = None;
        self.captured_geometry = None;
        if let Some(arrival) = self.arrival.take() {
            let _ = arrival
                .reply
                .try_send(Err("agent movement was canceled".into()));
        }
        if self.target.is_none()
            && self.contents.is_none()
            && self.keys.is_empty()
            && self.buttons.is_empty()
            && !self.visible
        {
            return;
        }
        let pointer = self.seat.get_pointer().unwrap();
        for button in self.buttons.drain() {
            if shared_pointer {
                continue;
            }
            pointer.button(
                state,
                &ButtonEvent {
                    button,
                    state: ButtonState::Released,
                    serial: SERIAL_COUNTER.next_serial(),
                    time: InputTime::now(),
                },
            );
        }
        pointer.frame(state);
        let keyboard = self.seat.get_keyboard().unwrap();
        keyboard.unset_grab(state);
        keyboard.set_focus(state, None, SERIAL_COUNTER.next_serial());
        pointer.unset_grab(state, SERIAL_COUNTER.next_serial(), InputTime::now());
        pointer.motion(
            state,
            None,
            &MotionEvent {
                location: pointer.current_location(),
                serial: SERIAL_COUNTER.next_serial(),
                time: InputTime::now(),
            },
        );
        pointer.frame(state);
        self.target = None;
        self.contents = None;
        if self.visible {
            self.visible = false;
            state.niri.queue_redraw_all();
        }
    }
}

impl Niri {
    pub fn agent_pointers_are_moving(&self, output: &Output) -> bool {
        self.agent_seats.values().any(|agent| {
            let Some(motion) = &agent.motion else {
                return false;
            };
            let current = agent.seat.get_pointer().unwrap().current_location();
            [current, motion.destination()].into_iter().any(|location| {
                self.output_under(location)
                    .is_some_and(|(under, _)| under == output)
            })
        })
    }

    pub fn render_agent_pointers<R: NiriRenderer>(
        &self,
        renderer: &mut R,
        output: &Output,
        push: &mut dyn FnMut(OutputRenderElements<R>),
    ) {
        if !matches!(self.lock_state, LockState::Unlocked) {
            return;
        }
        let origin = self
            .global_space
            .output_geometry(output)
            .unwrap()
            .loc
            .to_f64();
        let scale = Scale::from(output.current_scale().fractional_scale());
        for agent in self.agent_seats.values().filter(|agent| agent.visible) {
            let position = agent.seat.get_pointer().unwrap().current_location();
            if self
                .output_under(position)
                .is_none_or(|(under, _)| under != output)
            {
                continue;
            }
            let location = (position - origin).to_physical_precise_round(scale);
            match MemoryRenderBufferRenderElement::from_buffer(
                renderer,
                location,
                &agent.cursor,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                Ok(element) => push(crate::niri::PointerRenderElements::from(element).into()),
                Err(error) => warn!("error rendering agent cursor: {error:?}"),
            }
        }
    }
}

fn font_pointer_pixels(color: niri_config::Color) -> anyhow::Result<(Vec<u8>, (i32, i32))> {
    let decoder = png::Decoder::new(Cursor::new(include_bytes!(
        "../resources/agent-pointer.png"
    )));
    let mut reader = decoder.read_info()?;
    let mut pixels = vec![
        0;
        reader
            .output_buffer_size()
            .context("agent pointer size is unavailable")?
    ];
    let info = reader.next_frame(&mut pixels)?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight,
        "agent pointer must be an 8-bit RGBA image"
    );
    pixels.truncate(info.buffer_size());
    for pixel in pixels.chunks_exact_mut(4) {
        let channel = |value: f32| (value * 255.).round() as u8;
        let [red, green, blue] = [color.r, color.g, color.b].map(channel);
        let alpha = (f32::from(pixel[3]) * color.a).round() as u8;
        let premultiply = |channel: u8| (u16::from(channel) * u16::from(alpha) / 255) as u8;
        pixel.copy_from_slice(&[
            premultiply(blue),
            premultiply(green),
            premultiply(red),
            alpha,
        ]);
    }
    Ok((pixels, (info.width.try_into()?, info.height.try_into()?)))
}

fn font_pointer_buffer(color: niri_config::Color) -> anyhow::Result<MemoryRenderBuffer> {
    let (pixels, dimensions) = font_pointer_pixels(color)?;
    Ok(MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        dimensions,
        4,
        Transform::Normal,
        None,
    ))
}

/// Explains why a captured window no longer matches the window that is rendered now.
///
/// The message keeps the stable `capture window identity or rendered geometry changed` prefix and
/// appends the compared numbers, so a refusal can be told apart as a real move or resize, a
/// different client process, or a sub-pixel difference at a fractional output scale.
fn capture_window_mismatch(
    expected_pid: u32,
    actual_pid: Option<u32>,
    expected_rect: (f64, f64, f64, f64),
    actual_rect: (f64, f64, f64, f64),
) -> Option<String> {
    if actual_pid == Some(expected_pid) && actual_rect == expected_rect {
        return None;
    }
    Some(format!(
        "capture window identity or rendered geometry changed (pid expected {expected_pid}, \
         actual {actual_pid:?}; rect expected {expected_rect:?}, actual {actual_rect:?})"
    ))
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn targeted_commands_remain_guarded_until_pointer_arrival() {
        let button = AgentInput::Coordinated {
            revision: 7,
            command: Box::new(AgentInput::Targeted {
                window_id: 42,
                command: Box::new(AgentInput::Button {
                    button: 272,
                    state: AgentInputState::Pressed,
                }),
            }),
        };
        assert!(validate(&button).is_ok());
        assert!(deferred(&button));
        let invalid = AgentInput::Targeted {
            window_id: 42,
            command: Box::new(AgentInput::Reset),
        };
        assert!(validate(&invalid).is_err());
        let key = AgentInput::Targeted {
            window_id: 42,
            command: Box::new(AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            }),
        };
        assert!(validate(&key).is_ok());
        assert!(!deferred(&key));
    }

    #[test]
    fn nerd_font_pointer_has_transparency_and_premultiplied_color() {
        let color = niri_config::Color::from_rgba8_unpremul(122, 162, 247, 255);
        let (pixels, (width, height)) = font_pointer_pixels(color).unwrap();
        assert_eq!(height, 96);
        assert!(width > 0 && width < height);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [247, 162, 122, 255]));
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| (1..255).contains(&pixel[3])));
        assert!(pixels
            .chunks_exact(4)
            .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        font_pointer_buffer(color).unwrap();
    }
    #[test]
    fn capture_window_match_has_no_mismatch() {
        let rect = (80.0, 40.0, 762.0, 962.0);
        assert_eq!(capture_window_mismatch(7, Some(7), rect, rect), None);
    }

    #[test]
    fn capture_window_mismatch_reports_pid_and_rect_numbers() {
        let expected = (80.0, 40.0, 762.0, 962.0);
        let moved = (80.0, 41.5, 762.0, 962.0);
        let message = capture_window_mismatch(7, Some(7), expected, moved).unwrap();
        assert!(message.starts_with("capture window identity or rendered geometry changed"));
        assert!(message.contains("41.5"), "{message}");
        assert!(message.contains("pid expected 7, actual Some(7)"), "{message}");
        let other = capture_window_mismatch(7, Some(8), expected, expected).unwrap();
        assert!(other.contains("actual Some(8)"), "{other}");
        let gone = capture_window_mismatch(7, None, expected, expected).unwrap();
        assert!(gone.contains("actual None"), "{gone}");
        // Sub-pixel drift at a fractional output scale is still a refusal, and is now visible.
        let drift = capture_window_mismatch(7, Some(7), expected, (80.0, 40.0, 762.0 + 1e-9, 962.0)).unwrap();
        assert!(drift.contains("762.000000001"), "{drift}");
    }
}
