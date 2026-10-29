//! Finding the wl_output behind a connector name such as `eDP-1`. Rune names outputs by their
//! registry global, and the connector name only arrives in the output's `name` event (v4).
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_output::{self, WlOutput};
use wayland_client::protocol::wl_registry::WlRegistry;
use wayland_client::{Connection, Dispatch, QueueHandle};

#[derive(Default)]
struct Outputs {
    /// (registry global name, connector name)
    names: Vec<(u32, String)>,
}

impl Dispatch<WlRegistry, GlobalListContents> for Outputs {
    fn event(_: &mut Self, _: &WlRegistry, _: <WlRegistry as wayland_client::Proxy>::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<WlOutput, u32> for Outputs {
    fn event(state: &mut Self, _: &WlOutput, event: wl_output::Event, global: &u32, _: &Connection, _: &QueueHandle<Self>) {
        if let wl_output::Event::Name { name } = event {
            state.names.push((*global, name));
        }
    }
}

/// The registry global of the output called `connector`, or None when there is no such output
/// or the compositor is too old to report names.
pub fn global_for(connector: &str) -> Option<u32> {
    let connection = Connection::connect_to_env().ok()?;
    let (globals, mut queue) = registry_queue_init::<Outputs>(&connection).ok()?;
    let handle = queue.handle();
    let mut state = Outputs::default();
    let wanted: Vec<(u32, u32)> = globals
        .contents()
        .with_list(|list| list.iter().filter(|g| g.interface == "wl_output" && g.version >= 4).map(|g| (g.name, g.version)).collect());
    let _outputs: Vec<WlOutput> =
        wanted.iter().map(|&(name, version)| globals.registry().bind(name, version.min(4), &handle, name)).collect();
    queue.roundtrip(&mut state).ok()?;
    find(&state.names, connector)
}

fn find(names: &[(u32, String)], connector: &str) -> Option<u32> {
    names.iter().find(|(_, name)| name == connector).map(|(global, _)| *global)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_connector_name_picks_its_global() {
        let names = vec![(41, "eDP-1".to_owned()), (47, "HDMI-A-1".to_owned())];
        assert_eq!(find(&names, "HDMI-A-1"), Some(47));
        assert_eq!(find(&names, "DP-9"), None);
    }
}
