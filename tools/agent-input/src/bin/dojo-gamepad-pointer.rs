use std::collections::BTreeSet;
use std::io::{self, BufRead};
use std::time::Instant;

use wayland_client::protocol::{wl_pointer, wl_registry};
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct State {
    manager: Option<ZwlrVirtualPointerManagerV1>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zwlr_virtual_pointer_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(2), qh, ()));
            }
        }
    }
}

delegate_noop!(State: ignore ZwlrVirtualPointerManagerV1);
delegate_noop!(State: ignore ZwlrVirtualPointerV1);

#[derive(Debug, PartialEq)]
enum Command {
    Move(f64, f64),
    Button(u32, bool),
    Scroll(f64, f64),
}

fn parse(line: &str) -> Result<Command, &'static str> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 3 {
        return Err("expected move DX DY, button CODE 0|1, or scroll DX DY");
    }
    match fields[0] {
        "move" | "scroll" => {
            let x: f64 = fields[1].parse().map_err(|_| "invalid horizontal value")?;
            let y: f64 = fields[2].parse().map_err(|_| "invalid vertical value")?;
            if !x.is_finite() || !y.is_finite() || x.abs() > 10000.0 || y.abs() > 10000.0 {
                return Err("motion must be finite and at most 10000 per axis");
            }
            Ok(if fields[0] == "move" {
                Command::Move(x, y)
            } else {
                Command::Scroll(x, y)
            })
        }
        "button" => {
            let code = fields[1].parse().map_err(|_| "invalid button code")?;
            if !(272..=279).contains(&code) {
                return Err("button code must be a Linux mouse button from 272 through 279");
            }
            let pressed = match fields[2] {
                "0" => false,
                "1" => true,
                _ => return Err("button state must be 0 or 1"),
            };
            Ok(Command::Button(code, pressed))
        }
        _ => Err("unknown command"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::connect_to_env()?;
    let mut queue = connection.new_event_queue();
    let qh = queue.handle();
    connection.display().get_registry(&qh, ());
    let mut state = State::default();
    queue.roundtrip(&mut state)?;
    let pointer = state
        .manager
        .as_ref()
        .ok_or("virtual pointer unavailable")?
        .create_virtual_pointer(None, &qh, ());
    queue.roundtrip(&mut state)?;
    let start = Instant::now();
    let mut held = BTreeSet::new();
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        for line in io::stdin().lock().lines() {
            let line = line?;
            let command = match parse(&line) {
                Ok(command) => command,
                Err(error) => {
                    eprintln!("gamepad pointer: {error}");
                    continue;
                }
            };
            let time = start.elapsed().as_millis() as u32;
            match command {
                Command::Move(x, y) => pointer.motion(time, x, y),
                Command::Scroll(x, y) => {
                    pointer.axis_source(wl_pointer::AxisSource::Continuous);
                    if x != 0.0 {
                        pointer.axis(time, wl_pointer::Axis::HorizontalScroll, x);
                    }
                    if y != 0.0 {
                        pointer.axis(time, wl_pointer::Axis::VerticalScroll, y);
                    }
                }
                Command::Button(code, pressed) => {
                    let changed = if pressed {
                        held.insert(code)
                    } else {
                        held.remove(&code)
                    };
                    if changed {
                        pointer.button(
                            time,
                            code,
                            if pressed {
                                wl_pointer::ButtonState::Pressed
                            } else {
                                wl_pointer::ButtonState::Released
                            },
                        );
                    }
                }
            }
            pointer.frame();
            queue.roundtrip(&mut state)?;
        }
        Ok(())
    })();
    let time = start.elapsed().as_millis() as u32;
    for code in held {
        pointer.button(time, code, wl_pointer::ButtonState::Released);
    }
    pointer.frame();
    pointer.destroy();
    let cleanup = queue.roundtrip(&mut state);
    result?;
    cleanup?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Command, parse};

    #[test]
    fn parses_commands() {
        assert_eq!(parse("move -2.5 3"), Ok(Command::Move(-2.5, 3.0)));
        assert_eq!(parse("scroll 0 -1"), Ok(Command::Scroll(0.0, -1.0)));
        assert_eq!(parse("button 272 1"), Ok(Command::Button(272, true)));
        assert_eq!(parse("button 273 0"), Ok(Command::Button(273, false)));
    }

    #[test]
    fn rejects_invalid_input_before_injection() {
        for line in [
            "move NaN 0",
            "move inf 0",
            "scroll 0 -inf",
            "move 10001 0",
            "button 1 1",
            "button 272 2",
            "button 272 true",
            "move 1",
            "move 1 2 extra",
            "key 30 1",
        ] {
            assert!(parse(line).is_err(), "accepted {line}");
        }
    }
}
