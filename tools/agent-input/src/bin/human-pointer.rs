//! Inject primary-seat input into the explicitly selected nested Wayland session.
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
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        return Err("usage: human-pointer X Y OUTPUT_WIDTH OUTPUT_HEIGHT [click]".into());
    }
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
    pointer.motion_absolute(
        0,
        args[0].parse()?,
        args[1].parse()?,
        args[2].parse()?,
        args[3].parse()?,
    );
    pointer.frame();
    queue.roundtrip(&mut state)?;
    if args.get(4).is_some_and(|s| s == "click") {
        for button_state in [
            wl_pointer::ButtonState::Pressed,
            wl_pointer::ButtonState::Released,
        ] {
            pointer.button(0, 272, button_state);
            pointer.frame();
            queue.roundtrip(&mut state)?;
        }
    }
    pointer.destroy();
    queue.roundtrip(&mut state)?;
    Ok(())
}
