use niri_config::{Action, Config};
use niri_ipc::{AgentInputState, RemoteInput};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use wayland_client::protocol::{wl_keyboard, wl_surface::WlSurface};

use super::client::{ClientId, KeyboardEvent};
use super::fixture::Fixture;
use super::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};
use crate::remote_input::Origin;

const PEER: u64 = 7;

fn origin() -> Origin {
    Origin { user: "tag:venus".into(), ip: "100.71.60.101".into() }
}

fn press(evdev: u32) -> RemoteInput {
    RemoteInput::Key { evdev, state: AgentInputState::Pressed }
}

fn release(evdev: u32) -> RemoteInput {
    RemoteInput::Key { evdev, state: AgentInputState::Released }
}

fn set_up(config: Config) -> (Fixture, ClientId, WlSurface) {
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    let id = f.add_client();
    let window = f.client(id).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.roundtrip(id);
    let window = f.client(id).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.roundtrip(id);
    let _ = f.client(id).state.recent_keyboard_events(&surface);
    f.niri_state().begin_remote_session(PEER, origin());
    (f, id, surface)
}

fn keys(f: &mut Fixture, id: ClientId, surface: &WlSurface) -> Vec<(u32, wl_keyboard::KeyState)> {
    f.roundtrip(id);
    f.client(id)
        .state
        .recent_keyboard_events(surface)
        .filter_map(|event| match event {
            KeyboardEvent::Key { key, state } => Some((*key, *state)),
            _ => None,
        })
        .collect()
}

#[test]
fn remote_keys_reach_the_focused_window_as_the_real_keyboard() {
    let (mut f, id, surface) = set_up(Config::default());
    f.niri_state().deliver_remote_events(PEER, vec![press(30), release(30)]);
    assert_eq!(
        keys(&mut f, id, &surface),
        vec![(30, wl_keyboard::KeyState::Pressed), (30, wl_keyboard::KeyState::Released)]
    );
}

#[test]
fn remote_keys_above_255_are_delivered_too() {
    let (mut f, id, surface) = set_up(Config::default());
    f.niri_state().deliver_remote_events(PEER, vec![press(0x1d0), release(0x1d0)]);
    assert_eq!(
        keys(&mut f, id, &surface),
        vec![(0x1d0, wl_keyboard::KeyState::Pressed), (0x1d0, wl_keyboard::KeyState::Released)]
    );
}

#[test]
fn the_peers_binds_fire_from_remote_keys_and_the_bound_key_is_not_typed() {
    let mut config = Config::parse_mem(
        "\nbinds {\n    Mod+Ctrl+Q { close-window; }\n}\n",
    )
    .unwrap();
    for bind in &mut config.binds.0 {
        bind.action = Action::TestAction;
    }
    let (mut f, id, surface) = set_up(config);
    let before = f.niri().test_action_count;
    // Super (125), Ctrl (29), Q (16).
    f.niri_state().deliver_remote_events(PEER, vec![press(125), press(29), press(16), release(16), release(29), release(125)]);
    assert_eq!(f.niri().test_action_count - before, 1, "the bind ran once");
    let delivered = keys(&mut f, id, &surface);
    assert!(!delivered.iter().any(|(key, _)| *key == 16), "the bound key stays with the compositor: {delivered:?}");
}

#[test]
fn remote_pointer_motion_moves_the_real_cursor_and_buttons_click() {
    let (mut f, _id, _surface) = set_up(Config::default());
    let start = f.niri().seat.get_pointer().unwrap().current_location();
    f.niri_state().deliver_remote_events(PEER, vec![RemoteInput::Motion { dx: 40., dy: 25. }]);
    let end = f.niri().seat.get_pointer().unwrap().current_location();
    assert_eq!((end.x - start.x, end.y - start.y), (40., 25.));
}

#[test]
fn ending_a_session_lets_go_of_every_held_key() {
    let (mut f, id, surface) = set_up(Config::default());
    f.niri_state().deliver_remote_events(PEER, vec![press(30), press(31)]);
    let _ = keys(&mut f, id, &surface);
    f.niri_state().end_remote_input(PEER);
    let released = keys(&mut f, id, &surface);
    assert_eq!(released.len(), 2);
    assert!(released.iter().all(|(_, state)| *state == wl_keyboard::KeyState::Released), "{released:?}");
    assert!(f.niri().remote_input.is_empty());
}

#[test]
fn a_press_of_a_held_key_and_a_release_of_an_unheld_one_are_dropped() {
    let (mut f, id, surface) = set_up(Config::default());
    f.niri_state().deliver_remote_events(PEER, vec![release(30), press(30), press(30)]);
    assert_eq!(keys(&mut f, id, &surface), vec![(30, wl_keyboard::KeyState::Pressed)]);
}

#[test]
fn locking_the_screen_ends_remote_input_for_good() {
    let (mut f, id, surface) = set_up(Config::default());
    f.niri_state().deliver_remote_events(PEER, vec![press(30)]);
    let _ = keys(&mut f, id, &surface);
    let controller = f.client(id);
    let _lock = controller.state.session_lock_manager.as_ref().unwrap().lock(&controller.qh, ());
    f.double_roundtrip(id);
    f.niri_state().refresh_remote_input();
    assert!(f.niri().remote_input.is_empty());
    let refused = f.niri_state().request_remote_input(PEER, None, vec![press(31)]).unwrap_err();
    assert!(refused.starts_with("remote input ended:"), "{refused}");
}

#[test]
fn the_stop_chord_on_the_machines_own_keyboard_ends_remote_input_and_keeps_it_ended() {
    let (mut f, _id, _surface) = set_up(Config::default());
    // Hold Ctrl, Alt and Shift on the real keyboard, then press Esc.
    for code in [29_u32, 56, 42] {
        f.niri_state().process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::from_micros(0),
                code: Keycode::from(code + 8),
                state: KeyState::Pressed,
                count: 1,
            },
        });
    }
    f.niri_state().hardware_key_for_remote_input(1, true);
    assert!(f.niri().remote_input.is_empty());
    let refused = f.niri_state().request_remote_input(PEER, None, vec![press(31)]).unwrap_err();
    assert!(refused.starts_with("remote input ended:"), "{refused}");
}

#[test]
fn a_caller_that_cannot_be_identified_gets_nothing() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    let refused = f.niri_state().request_remote_input(9, None, vec![press(30)]).unwrap_err();
    assert!(refused.contains("could not identify the process"), "{refused}");
    assert!(f.niri().remote_input.is_empty());
}
