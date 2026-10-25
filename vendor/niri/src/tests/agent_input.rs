use niri_ipc::{AgentInput, AgentInputState};
use smithay::backend::input::{InputEvent, InputTime, KeyState, Keycode};
use wayland_client::protocol::{wl_keyboard, wl_surface::WlSurface};

use super::client::{ClientId, KeyboardEvent, PointerEvent};
use super::fixture::Fixture;
use super::test_input_backend::{TestInputBackend, TestKeyboardKeyEvent};

fn window(f: &mut Fixture) -> (ClientId, WlSurface, u64) {
    let client = f.add_client();
    let window = f.client(client).create_window();
    let surface = window.surface.clone();
    window.commit();
    f.double_roundtrip(client);
    let window = f.client(client).window(&surface);
    window.attach_new_buffer();
    window.ack_last_and_commit();
    f.double_roundtrip(client);
    let id = f.niri().layout.focus().unwrap().id().get();
    (client, surface, id)
}

#[test]
fn typing_preserves_focus_until_keyboard_idle() {
    let config = niri_config::Config::parse_mem("input { focus-follows-mouse; }").unwrap();
    let mut f = Fixture::with_config(config);
    f.add_output(1, (1920, 1080));
    f.niri().clock.set_complete_instantly(true);
    let (_, _, other) = window(&mut f);
    let (_, _, focused) = window(&mut f);
    let output = f.niri_output(1);
    let placements = f.niri().layout.capture_placements(&output).unwrap();
    let rect = placements
        .iter()
        .find(|(w, _, _)| w.id().get() == other)
        .unwrap()
        .1;
    let position = rect.loc
        + smithay::utils::Point::<f64, smithay::utils::Logical>::from((
            rect.size.w / 2.,
            rect.size.h / 2.,
        ));
    let under = f.niri().contents_under(position);
    assert!(under.window.is_some());
    f.niri()
        .clock
        .set_unadjusted(std::time::Duration::from_secs(1));
    for state in [KeyState::Pressed, KeyState::Released] {
        f.niri_state()
            .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
                event: TestKeyboardKeyEvent {
                    time: InputTime::now(),
                    code: Keycode::from(30_u32 + 8),
                    state,
                    count: u32::from(state == KeyState::Pressed),
                },
            });
    }
    f.niri().handle_focus_follows_mouse(&under);
    assert_eq!(
        f.niri().layout.focus().unwrap().id().get(),
        focused,
        "typing must preserve keyboard focus"
    );
    f.add_output(2, (1920, 1080));
    let second_output = f.niri_output(2);
    let other_output = crate::niri::PointContents {
        output: Some(second_output.clone()),
        ..Default::default()
    };
    f.niri().handle_focus_follows_mouse(&other_output);
    assert_eq!(f.niri().layout.active_output(), Some(&output));
    f.niri()
        .clock
        .set_unadjusted(std::time::Duration::from_secs(2));
    f.niri().handle_focus_follows_mouse(&under);
    assert_eq!(
        f.niri().layout.focus().unwrap().id().get(),
        other,
        "mouse focus must resume after keyboard idle"
    );
    f.niri().handle_focus_follows_mouse(&other_output);
    assert_eq!(f.niri().layout.active_output(), Some(&second_output));
}

fn keys(
    f: &mut Fixture,
    client: ClientId,
    surface: &WlSurface,
) -> Vec<(u32, wl_keyboard::KeyState)> {
    f.client(client)
        .state
        .recent_keyboard_events(surface)
        .filter_map(|event| match event {
            KeyboardEvent::Key { key, state } => Some((*key, *state)),
            _ => None,
        })
        .collect()
}

#[test]
fn independent_agent_and_human_keyboard_delivery() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (agent_client, agent_surface, target) = window(&mut f);
    let (human_client, human_surface, human_target) = window(&mut f);
    let primary_focus = f.niri().seat.get_keyboard().unwrap().current_focus();
    let primary_location = f.niri().seat.get_pointer().unwrap().current_location();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(agent_client);
    f.double_roundtrip(human_client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.double_roundtrip(agent_client);
    let _ = keys(&mut f, agent_client, &agent_surface);
    let _ = keys(&mut f, human_client, &human_surface);

    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::now(),
                code: Keycode::from(48_u32 + 8),
                state: KeyState::Pressed,
                count: 1,
            },
        });
    f.double_roundtrip(agent_client);
    f.double_roundtrip(human_client);
    assert_eq!(
        keys(&mut f, agent_client, &agent_surface),
        vec![(30, wl_keyboard::KeyState::Pressed)]
    );
    assert_eq!(
        keys(&mut f, human_client, &human_surface),
        vec![(48, wl_keyboard::KeyState::Pressed)]
    );
    assert_eq!(
        f.niri().seat.get_keyboard().unwrap().current_focus(),
        primary_focus
    );
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        primary_location
    );
    assert_eq!(f.niri().layout.focus().unwrap().id().get(), human_target);

    f.niri_state().destroy_agent(1);
    f.double_roundtrip(agent_client);
    assert_eq!(
        keys(&mut f, agent_client, &agent_surface),
        vec![(30, wl_keyboard::KeyState::Released)]
    );
    assert!(f.niri().agent_seats.is_empty());
    assert_eq!(
        f.niri().seat.get_keyboard().unwrap().current_focus(),
        primary_focus
    );
}

#[test]
fn agent_commands_validate_and_require_connection_ownership() {
    let mut f = Fixture::new();
    f.add_output(1, (1920, 1080));
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into()
            }
        )
        .is_err());
    f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "bad name".into()
            }
        )
        .is_err());
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    assert!(f
        .niri_state()
        .apply_agent_input(2, AgentInput::Focus { window_id: 1 })
        .is_err());
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Focus {
                window_id: u64::MAX
            }
        )
        .is_err());
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed
            }
        )
        .is_err());
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output: "missing".into(),
                x: 0.,
                y: 0.
            }
        )
        .is_err());
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output: "missing".into(),
                x: f64::NAN,
                y: 0.
            }
        )
        .is_err());
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    f.niri_state()
        .apply_agent_input(1, AgentInput::Destroy)
        .unwrap();
    f.niri_state()
        .apply_agent_input(1, AgentInput::Destroy)
        .unwrap();
}

#[test]
fn jump_places_the_agent_pointer_at_once_where_move_animates() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let output = f.niri_output(1).name();
    let location = |f: &mut Fixture| {
        f.niri().agent_seats[&1]
            .seat
            .get_pointer()
            .unwrap()
            .current_location()
    };
    f.niri_state()
        .apply_agent_input(1, AgentInput::Create { name: "jump".into() })
        .ok();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Jump {
                output: output.clone(),
                x: 100.,
                y: 100.,
            },
        )
        .unwrap();
    // The seat is shown at its first position whichever way it got there.
    assert_eq!(location(&mut f), (100., 100.).into());

    // A move to somewhere else is an animation: Niri says so and the pointer has not arrived.
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Move {
                    output: output.clone(),
                    x: 900.,
                    y: 600.,
                }
            )
            .unwrap(),
        niri_ipc::Response::AgentInputMoving
    ));
    assert_ne!(location(&mut f), (900., 600.).into());

    // A jump is there already, with nothing to wait for, and it ends the animation under way.
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Jump {
                    output: output.clone(),
                    x: 500.,
                    y: 400.,
                }
            )
            .unwrap(),
        niri_ipc::Response::Handled
    ));
    assert_eq!(location(&mut f), (500., 400.).into());
    let output_handle = f.niri_output(1);
    assert!(!f.niri().agent_pointers_are_moving(&output_handle));

    // The same limits as a move.
    for bad in [
        AgentInput::Jump { output: "missing".into(), x: 0., y: 0. },
        AgentInput::Jump { output: output.clone(), x: -1., y: 0. },
        AgentInput::Jump { output: output.clone(), x: 5000., y: 0. },
        AgentInput::Jump { output, x: f64::NAN, y: 0. },
    ] {
        assert!(f.niri_state().apply_agent_input(1, bad).is_err());
    }
    f.niri_state().destroy_agent(1);
}

#[test]
fn locking_releases_agent_keys_and_rejects_typing() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    let _ = keys(&mut f, client, &surface);
    let controller = f.client(client);
    let _lock = controller
        .state
        .session_lock_manager
        .as_ref()
        .unwrap()
        .lock(&controller.qh, ());
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![(30, wl_keyboard::KeyState::Released)]
    );
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed
            }
        )
        .is_err());
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    f.niri_state()
        .apply_agent_input(1, AgentInput::Destroy)
        .unwrap();
    assert!(f.niri().agent_seats.is_empty());
}

#[test]
fn unmapped_target_releases_held_input_and_clears_agent_focus() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    let _ = keys(&mut f, client, &surface);
    surface.attach(None, 0, 0);
    surface.commit();
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![(30, wl_keyboard::KeyState::Released)]
    );
    let agent = &f.niri().agent_seats[&1];
    assert!(agent.seat.get_keyboard().unwrap().current_focus().is_none());
    assert!(agent.seat.get_pointer().unwrap().current_focus().is_none());
    assert!(f
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed
            }
        )
        .is_err());
}

#[test]
fn experimental_seats_hide_incompatible_protocol_globals() {
    let mut standard = Fixture::new();
    let standard_client = standard.add_client();
    let standard_globals = &standard.client(standard_client).state.globals;
    for interface in [
        "wp_cursor_shape_manager_v1",
        "zwp_virtual_keyboard_manager_v1",
        "zwp_input_method_manager_v2",
    ] {
        assert!(
            standard_globals
                .iter()
                .any(|global| global.interface == interface),
            "default mode must advertise {interface}"
        );
    }
    let mut experimental = Fixture::with_agent_input();
    let client = experimental.add_client();
    experimental
        .niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    experimental.double_roundtrip(client);
    let globals = &experimental.client(client).state.globals;
    assert!(globals
        .iter()
        .any(|global| global.interface == "wp_cursor_shape_manager_v1"));
    for interface in [
        "zwp_virtual_keyboard_manager_v1",
        "zwp_input_method_manager_v2",
    ] {
        assert!(
            !globals.iter().any(|global| global.interface == interface),
            "experimental mode must hide {interface}"
        );
    }
    assert!(globals.iter().any(|global| global.interface == "wl_seat"));
}

#[test]
fn destroying_agent_frames_button_release_before_pointer_leave() {
    let mut f = Fixture::with_agent_input();
    f.niri().clock.set_complete_instantly(true);
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    let window = f.client(client).window(&surface);
    window.set_size(1000, 1000);
    window.commit();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "test".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.double_roundtrip(client);
    let output = f.niri_output(1).name();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output,
                x: 500.,
                y: 500.,
            },
        )
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Button {
                button: 272,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 42,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    assert!(f
        .client(client)
        .state
        .pointer_events
        .contains(&PointerEvent::Button {
            button: 272,
            state: wayland_client::protocol::wl_pointer::ButtonState::Pressed
        }));
    f.client(client).state.pointer_events.clear();
    f.niri_state().destroy_agent(1);
    f.double_roundtrip(client);
    assert_eq!(
        f.client(client).state.pointer_events,
        vec![
            PointerEvent::Button {
                button: 272,
                state: wayland_client::protocol::wl_pointer::ButtonState::Released
            },
            PointerEvent::Frame,
            PointerEvent::Leave,
            PointerEvent::Frame,
        ]
    );
}

#[test]
fn agent_click_waits_for_arrival_and_reset_cancels_wait() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    let window = f.client(client).window(&surface);
    window.set_size(1000, 1000);
    window.commit();
    f.double_roundtrip(client);
    let output = f.niri_output(1).name();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "motion".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.double_roundtrip(client);
    let human = f.niri().seat.get_pointer().unwrap().current_location();
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Move {
                    output: output.clone(),
                    x: 500.,
                    y: 500.,
                }
            )
            .unwrap(),
        niri_ipc::Response::AgentInputMoving
    ));
    let (tx, rx) = async_channel::bounded(1);
    f.niri_state().request_agent_input(
        1,
        AgentInput::Button {
            button: 272,
            state: AgentInputState::Pressed,
        },
        tx,
    );
    assert!(rx.try_recv().is_err());
    f.niri().clock.set_complete_instantly(true);
    f.niri_state().refresh_agents();
    assert!(matches!(
        rx.try_recv().unwrap().unwrap(),
        niri_ipc::Response::Handled
    ));
    let output_handle = f.niri_output(1);
    assert!(!f.niri().agent_pointers_are_moving(&output_handle));
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        human
    );
    assert_eq!(
        f.niri().agent_seats[&1]
            .seat
            .get_pointer()
            .unwrap()
            .current_location(),
        (500., 500.).into()
    );
    f.double_roundtrip(client);
    assert!(f
        .client(client)
        .state
        .pointer_events
        .iter()
        .any(|event| matches!(event, PointerEvent::Button { button: 272, .. })));
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    f.niri().clock.set_complete_instantly(false);
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output,
                x: 700.,
                y: 700.,
            },
        )
        .unwrap();
    let (tx, rx) = async_channel::bounded(1);
    f.niri_state().request_agent_input(1, AgentInput::Wait, tx);
    assert!(rx.try_recv().is_err());
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    assert!(rx.try_recv().unwrap().unwrap_err().contains("canceled"));
    let output_handle = f.niri_output(1);
    assert!(!f.niri().agent_pointers_are_moving(&output_handle));
    f.niri_state().destroy_agent(1);
    assert!(f.niri().agent_seats.is_empty());
}

#[test]
fn shared_client_receives_input_and_human_takeover_releases_it() {
    let mut f = Fixture::with_agent_input();
    f.niri().clock.set_complete_instantly(true);
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    f.client(client).state.primary_seat_only = true;
    let window = f.client(client).window(&surface);
    window.set_size(1000, 1000);
    window.commit();
    f.double_roundtrip(client);
    let primary_location = f.niri().seat.get_pointer().unwrap().current_location();
    let primary_focus = f.niri().seat.get_keyboard().unwrap().current_focus();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "shared".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    assert_eq!(f.client(client).state.seats.len(), 1);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.double_roundtrip(client);
    let _ = keys(&mut f, client, &surface);
    let output = f.niri_output(1).name();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output,
                x: 500.,
                y: 500.,
            },
        )
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Button {
                button: 272,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 42,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![(42, wl_keyboard::KeyState::Pressed)]
    );
    assert!(f
        .client(client)
        .state
        .pointer_events
        .contains(&PointerEvent::Button {
            button: 272,
            state: wayland_client::protocol::wl_pointer::ButtonState::Pressed
        }));
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        primary_location
    );
    assert_eq!(
        f.niri().seat.get_keyboard().unwrap().current_focus(),
        primary_focus
    );
    f.client(client).state.pointer_events.clear();
    // Entering this application transfers its input before any human event is delivered.
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: super::test_input_backend::TestPointerMotionAbsoluteEvent { x: 500., y: 500. },
        });
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![(42, wl_keyboard::KeyState::Released)]
    );
    assert!(f
        .client(client)
        .state
        .pointer_events
        .contains(&PointerEvent::Button {
            button: 272,
            state: wayland_client::protocol::wl_pointer::ButtonState::Released
        }));
    assert!(f.client(client).state.pointer_events.starts_with(&[
        PointerEvent::Button {
            button: 272,
            state: wayland_client::protocol::wl_pointer::ButtonState::Released
        },
        PointerEvent::Frame,
        PointerEvent::Leave,
        PointerEvent::Frame,
    ]));
    assert!(f.niri().agent_seats[&1]
        .seat
        .get_pointer()
        .unwrap()
        .current_focus()
        .is_none());
    let snapshot = f.niri_state().input_coordination_snapshot();
    assert_eq!(snapshot.revision, 1);
    assert_eq!(snapshot.human_owned_windows, vec![target]);
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(1, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::AgentInputPaused { .. }
    ));
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Coordinated {
                    revision: 0,
                    command: Box::new(AgentInput::Wait)
                }
            )
            .unwrap(),
        niri_ipc::Response::AgentInputPaused { .. }
    ));
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: super::test_input_backend::TestPointerMotionAbsoluteEvent { x: 1910., y: 1070. },
        });
    assert_eq!(f.niri_state().input_coordination_snapshot().revision, 2);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.niri_state().destroy_agent(1);
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![
            (30, wl_keyboard::KeyState::Pressed),
            (30, wl_keyboard::KeyState::Released)
        ]
    );
}

#[test]
fn stationary_human_pointer_releases_ownership_when_window_unmaps() {
    let mut f = Fixture::with_agent_input();
    f.niri().clock.set_complete_instantly(true);
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    let window = f.client(client).window(&surface);
    window.set_size(1000, 1000);
    window.commit();
    f.double_roundtrip(client);
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: super::test_input_backend::TestPointerMotionAbsoluteEvent { x: 500., y: 500. },
        });
    let before = f.niri_state().input_coordination_snapshot();
    assert_eq!(before.human_owned_windows, vec![target]);
    let location = f.niri().seat.get_pointer().unwrap().current_location();

    surface.attach(None, 0, 0);
    surface.commit();
    f.double_roundtrip(client);
    f.niri_state().update_pointer_contents();

    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        location
    );
    assert!(f.niri().input_coordination.pointer.is_none());
    let after = f.niri_state().input_coordination_snapshot();
    assert_eq!(after.revision, before.revision + 1);
    assert!(after.human_owned_windows.is_empty());
}

#[test]
fn native_seats_keep_independent_held_keys_in_one_client() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, _, target) = window(&mut f);
    for peer in [1, 2] {
        f.niri_state()
            .apply_agent_input(
                peer,
                AgentInput::Create {
                    name: format!("native-{peer}"),
                },
            )
            .unwrap();
        f.double_roundtrip(client);
        f.niri_state()
            .apply_agent_input(peer, AgentInput::Focus { window_id: target })
            .unwrap();
        f.double_roundtrip(client);
    }
    for (peer, evdev) in [(1, 42), (2, 48)] {
        let response = f
            .niri_state()
            .apply_agent_input(
                peer,
                AgentInput::Key {
                    evdev,
                    state: AgentInputState::Pressed,
                },
            )
            .unwrap();
        assert!(
            matches!(response, niri_ipc::Response::Handled),
            "peer {peer}: {response:?}"
        );
    }
    assert_ne!(
        f.niri().agent_seats[&1]
            .seat
            .get_keyboard()
            .unwrap()
            .modifier_state()
            .serialized
            .depressed,
        0
    );
    assert_eq!(
        f.niri().agent_seats[&2]
            .seat
            .get_keyboard()
            .unwrap()
            .modifier_state()
            .serialized
            .depressed,
        0
    );
}

#[test]
fn agents_serialize_held_input_in_one_application() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, _, target) = window(&mut f);
    for peer in [1, 2] {
        f.niri_state()
            .apply_agent_input(
                peer,
                AgentInput::Create {
                    name: format!("agent{peer}"),
                },
            )
            .unwrap();
    }
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(2, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::AgentInputPaused { .. }
    ));
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 30,
                state: AgentInputState::Released,
            },
        )
        .unwrap();
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(2, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::Handled
    ));
}

#[test]
fn human_typing_takes_over_and_held_keys_preserve_ownership() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    let (client, surface, target) = window(&mut f);
    f.client(client).state.primary_seat_only = true;
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "typing".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    f.niri_state()
        .apply_agent_input(1, AgentInput::Focus { window_id: target })
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Key {
                evdev: 42,
                state: AgentInputState::Pressed,
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    let _ = keys(&mut f, client, &surface);
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::now(),
                code: Keycode::from(48_u32 + 8),
                state: KeyState::Pressed,
                count: 1,
            },
        });
    f.double_roundtrip(client);
    assert_eq!(
        keys(&mut f, client, &surface),
        vec![
            (42, wl_keyboard::KeyState::Released),
            (48, wl_keyboard::KeyState::Pressed)
        ]
    );
    assert_eq!(
        f.niri()
            .seat
            .get_keyboard()
            .unwrap()
            .modifier_state()
            .serialized
            .depressed,
        0
    );
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: super::test_input_backend::TestPointerMotionAbsoluteEvent { x: 1910., y: 1070. },
        });
    assert_eq!(
        f.niri_state()
            .input_coordination_snapshot()
            .human_owned_windows,
        vec![target]
    );
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::now(),
                code: Keycode::from(48_u32 + 8),
                state: KeyState::Released,
                count: 0,
            },
        });
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
            event: super::test_input_backend::TestPointerMotionAbsoluteEvent { x: 1910., y: 1070. },
        });
    assert!(f
        .niri_state()
        .input_coordination_snapshot()
        .human_owned_windows
        .is_empty());
}

#[test]
fn explicit_handoff_allows_agent_then_human_motion_takes_back_control() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    f.niri().clock.set_complete_instantly(true);
    let (client, surface, target) = window(&mut f);
    let window = f.client(client).window(&surface);
    window.set_size(1000, 1000);
    window.commit();
    f.double_roundtrip(client);
    let motion = |f: &mut Fixture| {
        f.niri_state()
            .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
                event: super::test_input_backend::TestPointerMotionAbsoluteEvent {
                    x: 500.,
                    y: 500.,
                },
            });
    };
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "handoff".into(),
            },
        )
        .unwrap();
    f.double_roundtrip(client);
    motion(&mut f);
    assert_eq!(
        f.niri_state()
            .input_coordination_snapshot()
            .human_owned_windows,
        vec![target]
    );
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(1, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::AgentInputPaused { .. }
    ));
    let location = f.niri().seat.get_pointer().unwrap().current_location();
    let focus = f.niri().seat.get_keyboard().unwrap().current_focus();
    f.niri_state().yield_human_input().unwrap();
    assert_eq!(
        f.niri().seat.get_pointer().unwrap().current_location(),
        location
    );
    assert_eq!(f.niri().seat.get_keyboard().unwrap().current_focus(), focus);
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(1, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::Handled
    ));
    motion(&mut f);
    assert!(matches!(
        f.niri_state()
            .apply_agent_input(1, AgentInput::Focus { window_id: target })
            .unwrap(),
        niri_ipc::Response::AgentInputPaused { .. }
    ));
    f.niri_state()
        .process_input_event(InputEvent::<TestInputBackend>::Keyboard {
            event: TestKeyboardKeyEvent {
                time: InputTime::now(),
                code: Keycode::from(48_u32 + 8),
                state: KeyState::Pressed,
                count: 1,
            },
        });
    assert!(f.niri_state().yield_human_input().is_err());
}

#[test]
fn agent_motion_queues_current_and_destination_outputs_without_animating_others() {
    use crate::niri::RedrawState;
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    f.add_output(2, (1920, 1080));
    f.add_output(3, (1920, 1080));
    let first = f.niri_output(1);
    let second = f.niri_output(2);
    let third = f.niri_output(3);
    f.niri().global_space.map_output(&first, (0, 0));
    f.niri().global_space.map_output(&second, (3000, 0));
    f.niri().global_space.map_output(&third, (6000, 0));
    let human_pointer = f.niri().seat.get_pointer().unwrap();
    human_pointer.motion(
        f.niri_state(),
        None,
        &smithay::input::pointer::MotionEvent {
            location: (100., 100.).into(),
            serial: smithay::utils::SERIAL_COUNTER.next_serial(),
            time: InputTime::now(),
        },
    );
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "outputs".into(),
            },
        )
        .unwrap();
    for state in f.niri().output_state.values_mut() {
        state.redraw_state = RedrawState::Idle;
    }
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output: first.name(),
                x: 500.,
                y: 500.,
            },
        )
        .unwrap();
    assert!(f.niri().agent_pointers_are_moving(&first));
    assert!(!f.niri().agent_pointers_are_moving(&second));
    assert!(!f.niri().agent_pointers_are_moving(&third));
    assert!(matches!(
        f.niri().output_state[&first].redraw_state,
        RedrawState::Queued
    ));
    assert!(matches!(
        f.niri().output_state[&second].redraw_state,
        RedrawState::Idle
    ));
    assert!(matches!(
        f.niri().output_state[&third].redraw_state,
        RedrawState::Idle
    ));

    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output: second.name(),
                x: 500.,
                y: 500.,
            },
        )
        .unwrap();
    assert!(f.niri().agent_pointers_are_moving(&first));
    assert!(f.niri().agent_pointers_are_moving(&second));
    assert!(!f.niri().agent_pointers_are_moving(&third));
    assert!(matches!(
        f.niri().output_state[&second].redraw_state,
        RedrawState::Queued
    ));
    assert!(matches!(
        f.niri().output_state[&third].redraw_state,
        RedrawState::Idle
    ));

    // A gap between outputs must retain the destination's frame source.
    f.niri_state()
        .with_agent(1, |state, agent| {
            agent.seat.get_pointer().unwrap().motion(
                state,
                None,
                &smithay::input::pointer::MotionEvent {
                    location: (2500., 500.).into(),
                    serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                    time: InputTime::now(),
                },
            );
        })
        .unwrap();
    assert!(!f.niri().agent_pointers_are_moving(&first));
    assert!(f.niri().agent_pointers_are_moving(&second));
    assert!(!f.niri().agent_pointers_are_moving(&third));

    f.niri().clock.set_complete_instantly(true);
    f.niri_state().refresh_agents();
    for output in [&first, &second, &third] {
        assert!(!f.niri().agent_pointers_are_moving(output));
    }
    assert_eq!(
        f.niri().agent_seats[&1]
            .seat
            .get_pointer()
            .unwrap()
            .current_location(),
        (3500., 500.).into()
    );
    assert!(matches!(
        f.niri().output_state[&third].redraw_state,
        RedrawState::Idle
    ));
    f.niri_state()
        .apply_agent_input(1, AgentInput::Reset)
        .unwrap();
    assert!(!f.niri().agent_pointers_are_moving(&second));
}

#[test]
fn preexisting_human_ownership_pauses_targeted_pointer_input_without_effects() {
    for press in [false, true] {
        let mut f = Fixture::with_agent_input();
        f.add_output(1, (1920, 1080));
        f.niri().clock.set_complete_instantly(true);
        let (client, surface, target) = window(&mut f);
        let window = f.client(client).window(&surface);
        window.set_size(1000, 1000);
        window.commit();
        f.double_roundtrip(client);
        f.niri_state()
            .apply_agent_input(
                1,
                AgentInput::Create {
                    name: "preowned".into(),
                },
            )
            .unwrap();
        let output = f.niri().global_space.outputs().next().unwrap().name();
        let move_command = AgentInput::Move {
            output,
            x: 500.,
            y: 500.,
        };
        if press {
            f.niri_state()
                .apply_agent_input(1, move_command.clone())
                .unwrap();
        }
        f.niri_state()
            .process_input_event(InputEvent::<TestInputBackend>::PointerMotionAbsolute {
                event: super::test_input_backend::TestPointerMotionAbsoluteEvent {
                    x: 500.,
                    y: 500.,
                },
            });
        f.double_roundtrip(client);
        assert_eq!(
            f.niri_state()
                .input_coordination_snapshot()
                .human_owned_windows,
            vec![target]
        );
        f.client(client).state.pointer_events.clear();
        let pointer = f.niri().agent_seats[&1].seat.get_pointer().unwrap();
        let before = pointer.current_location();
        let command = if press {
            AgentInput::Button {
                button: 272,
                state: AgentInputState::Pressed,
            }
        } else {
            move_command
        };
        let response = f
            .niri_state()
            .apply_agent_input(
                1,
                AgentInput::Targeted {
                    window_id: target,
                    command: Box::new(command),
                },
            )
            .unwrap();
        assert!(
            matches!(response, niri_ipc::Response::AgentInputPaused { .. }),
            "press={press}: {response:?}"
        );
        assert_eq!(pointer.current_location(), before);
        f.double_roundtrip(client);
        assert!(!f
            .client(client)
            .state
            .pointer_events
            .iter()
            .any(|event| matches!(event, PointerEvent::Button { .. })));
    }
}

#[test]
fn captured_unheld_pointer_motion_never_focuses_another_application() {
    let mut f = Fixture::with_agent_input();
    f.add_output(1, (1920, 1080));
    f.niri().clock.set_complete_instantly(true);
    let (other_client, _, other_id) = window(&mut f);
    let (target_client, _, target_id) = window(&mut f);
    let output = f.niri_output(1);
    let revision = f.niri_state().input_coordination_snapshot().revision;
    let placements = f.niri().layout.capture_placements(&output).unwrap();
    let other_rect = placements
        .iter()
        .find(|(window, _, _)| window.id().get() == other_id)
        .unwrap()
        .1;
    let (target, target_rect, _) = placements
        .iter()
        .find(|(window, _, _)| window.id().get() == target_id)
        .unwrap();
    let stat = std::fs::read_to_string(format!("/proc/{}/stat", std::process::id())).unwrap();
    let geometry = niri_ipc::CapturedInputGeometry {
        window_id: target_id,
        pid: target.credentials().unwrap().pid as u32,
        compositor_pid: std::process::id(),
        compositor_start_time: stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap(),
        coordination_revision: revision,
        output: output.name(),
        output_geometry: crate::utils::logical_output(&output),
        rect: (
            target_rect.loc.x,
            target_rect.loc.y,
            target_rect.size.w,
            target_rect.size.h,
        ),
    };
    let destination = (
        target_rect.loc.x + target_rect.size.w / 2.,
        target_rect.loc.y + target_rect.size.h / 2.,
    );
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Create {
                name: "captured-path".into(),
            },
        )
        .unwrap();
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Move {
                output: output.name(),
                x: other_rect.loc.x + other_rect.size.w / 2.,
                y: other_rect.loc.y + other_rect.size.h / 2.,
            },
        )
        .unwrap();
    let pointer = f.niri().agent_seats[&1].seat.get_pointer().unwrap();
    assert!(
        pointer.current_focus().is_some(),
        "legacy move should still reach another application"
    );
    f.niri().clock.set_complete_instantly(false);
    f.niri_state()
        .apply_agent_input(
            1,
            AgentInput::Captured {
                geometry,
                command: Box::new(AgentInput::Move {
                    output: output.name(),
                    x: destination.0,
                    y: destination.1,
                }),
            },
        )
        .unwrap();
    f.double_roundtrip(other_client);
    assert!(
        pointer.current_focus().is_none(),
        "captured animation delivered pointer focus outside its target"
    );
    f.niri().clock.set_complete_instantly(true);
    f.niri_state().refresh_agents();
    f.double_roundtrip(target_client);
    let focus = pointer
        .current_focus()
        .expect("captured animation must arrive at its target");
    let root = f.niri().find_root_shell_surface(&focus);
    assert_eq!(
        f.niri()
            .layout
            .find_window_and_output(&root)
            .unwrap()
            .0
            .id()
            .get(),
        target_id
    );
}
