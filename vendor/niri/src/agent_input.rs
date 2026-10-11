//! IPC-controlled logical seats. Each controller owns its seat state; dispatch selects that
//! state synchronously so Smithay can resolve handles without retaining retired seats forever.
//! DOJO_AGENT_INPUT=1 disables cursor shapes, virtual keyboards, and input methods;
//! agent input has no client cursors, popup grabs, interactive move/resize, DnD, or X11 support.

use std::collections::HashSet;
use std::io::Cursor;
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

use crate::niri::{LockState, Niri, OutputRenderElements, State};
use crate::render_helpers::renderer::NiriRenderer;

pub struct AgentSeat {
    pub seat: Seat<State>,
    state: SeatState<State>,
    target: Option<WlSurface>,
    contents: Option<(WlSurface, Point<f64, Logical>)>,
    keys: HashSet<u32>,
    buttons: HashSet<u32>,
    visible: bool,
    cursor: MemoryRenderBuffer,
}

pub fn enabled() -> bool {
    std::env::var_os("DOJO_AGENT_INPUT").is_some_and(|value| value == "1")
}

fn validate(command: &AgentInput) -> Result<(), String> {
    match command {
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
        AgentInput::Move { x, y, .. } if !x.is_finite() || !y.is_finite() => {
            return Err("pointer coordinates must be finite".into());
        }
        AgentInput::Button { button, .. } if !(0x110..=0x117).contains(button) => {
            return Err("button must be a Linux mouse button code between 272 and 279".into());
        }
        AgentInput::Key { evdev, .. } if !(1..=255).contains(evdev) => {
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

impl State {
    pub fn apply_agent_input(
        &mut self,
        peer: u64,
        command: AgentInput,
    ) -> Result<Response, String> {
        validate(&command)?;
        if !self.niri.agent_input_enabled {
            return Err("agent input requires DOJO_AGENT_INPUT=1 on the compositor".into());
        }
        if !matches!(command, AgentInput::Reset | AgentInput::Destroy)
            && !matches!(self.niri.lock_state, LockState::Unlocked)
        {
            return Err("agent input is suspended while the session is locking or locked".into());
        }
        self.refresh_agents();
        if let AgentInput::Create { name } = command {
            if self.niri.agent_seats.contains_key(&peer) {
                return Err("this connection already owns an agent seat".into());
            }
            if self.niri.agent_seats.len() >= 16 {
                return Err("too many live agent controllers".into());
            }
            let cursor = font_pointer_buffer()
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
                    seat,
                    state,
                    target: None,
                    contents: None,
                    keys: HashSet::new(),
                    buttons: HashSet::new(),
                    visible: false,
                    cursor,
                },
            );
            return Ok(Response::AgentInputCreated { seat_name });
        }
        if matches!(command, AgentInput::Destroy) {
            self.destroy_agent(peer);
            return Ok(Response::Handled);
        }
        self.with_agent(peer, |this, agent| this.dispatch_agent(agent, command))??;
        Ok(Response::Handled)
    }

    fn with_agent<T>(
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

    fn dispatch_agent(&mut self, agent: &mut AgentSeat, command: AgentInput) -> Result<(), String> {
        let pointer = agent.seat.get_pointer().unwrap();
        let keyboard = agent.seat.get_keyboard().unwrap();
        match command {
            AgentInput::Focus { window_id } => {
                let mut target = None;
                self.niri.layout.with_windows(|mapped, output, _, _| {
                    if mapped.id().get() == window_id && output.is_some() {
                        target = Some(mapped.toplevel().wl_surface().clone());
                    }
                });
                let target = target.ok_or_else(|| "no mapped window with that ID".to_string())?;
                agent.set_target(self, target);
            }
            AgentInput::Move { output, x, y } => {
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
                let under = self.niri.contents_under(location);
                agent.contents = under.surface.clone();
                pointer.motion(
                    self,
                    under.surface,
                    &MotionEvent {
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: InputTime::now(),
                    },
                );
                pointer.frame(self);
                agent.visible = true;
            }
            AgentInput::Button { button, state } => {
                let pressed = state == AgentInputState::Pressed;
                if pressed {
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
                    |_, _, _| FilterResult::Forward,
                );
            }
            AgentInput::Axis {
                horizontal,
                vertical,
            } => {
                if !agent.visible || pointer.current_focus().is_none() {
                    return Err("move the agent pointer onto a client surface first".into());
                }
                pointer.axis(
                    self,
                    AxisFrame::new(InputTime::now())
                        .source(AxisSource::Continuous)
                        .value(Axis::Horizontal, horizontal)
                        .value(Axis::Vertical, vertical),
                );
                pointer.frame(self);
            }
            AgentInput::Reset => agent.reset(self),
            AgentInput::Create { .. } | AgentInput::Destroy => unreachable!(),
        }
        self.niri.queue_redraw_all();
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
                if agent.visible {
                    let pointer = agent.seat.get_pointer().unwrap();
                    let location = pointer.current_location();
                    if this.niri.output_under(location).is_none() {
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
                    agent.contents = under.surface.clone();
                    pointer.motion(
                        this,
                        under.surface,
                        &MotionEvent {
                            location,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: InputTime::now(),
                        },
                    );
                    pointer.frame(this);
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
    fn set_target(&mut self, state: &mut State, target: WlSurface) {
        if self.target.as_ref() == Some(&target) {
            return;
        }
        self.release_keys(state);
        self.target = Some(target.clone());
        self.seat.get_keyboard().unwrap().set_focus(
            state,
            Some(target),
            SERIAL_COUNTER.next_serial(),
        );
    }

    fn release_keys(&mut self, state: &mut State) {
        let keyboard = self.seat.get_keyboard().unwrap();
        for evdev in self.keys.drain() {
            keyboard.input::<(), _>(
                state,
                Keycode::from(evdev + 8),
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_, _, _| FilterResult::Forward,
            );
        }
    }

    fn reset(&mut self, state: &mut State) {
        if self.target.is_none()
            && self.contents.is_none()
            && self.keys.is_empty()
            && self.buttons.is_empty()
            && !self.visible
        {
            return;
        }
        self.release_keys(state);
        let pointer = self.seat.get_pointer().unwrap();
        for button in self.buttons.drain() {
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

fn font_pointer_pixels() -> anyhow::Result<(Vec<u8>, (i32, i32))> {
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
        let [red, green, blue, alpha] = [pixel[0], pixel[1], pixel[2], pixel[3]];
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

fn font_pointer_buffer() -> anyhow::Result<MemoryRenderBuffer> {
    let (pixels, dimensions) = font_pointer_pixels()?;
    Ok(MemoryRenderBuffer::from_slice(
        &pixels,
        Fourcc::Argb8888,
        dimensions,
        4,
        Transform::Normal,
        None,
    ))
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    #[test]
    fn nerd_font_pointer_has_transparency_and_premultiplied_color() {
        let (pixels, (width, height)) = font_pointer_pixels().unwrap();
        assert_eq!(height, 96);
        assert!(width > 0 && width < height);
        assert_eq!(pixels.len(), (width * height * 4) as usize);
        assert!(pixels.chunks_exact(4).any(|pixel| pixel[3] == 0));
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| pixel == [240, 200, 40, 255]));
        assert!(pixels
            .chunks_exact(4)
            .any(|pixel| (1..255).contains(&pixel[3])));
        assert!(pixels
            .chunks_exact(4)
            .all(|pixel| pixel[..3].iter().all(|channel| *channel <= pixel[3])));
        font_pointer_buffer().unwrap();
    }
}
